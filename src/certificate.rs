
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use rcgen::{Certificate, CertificateParams, DistinguishedName};
use tokio::time::sleep;
use reqwest;

use instant_acme::{
    Account, AccountCredentials, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus
};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem, time::ASN1Time};

use crate::{error::Error, server::{get_server_file_path, get_server_file_string, write_server_file}, tools::log::{log_info, LogServiceType}, Result};

pub async fn dns_certify(domain: &str, duck_dns: &str) -> Result<(PathBuf, PathBuf)> {
    log_info(LogServiceType::Register, format!("Getting https certificate"));
    const ACCOUNT_FILENAME: &str = "letsencrypt_account.json";
    const PUBLIC_FILENAME: &str = "cert_chain.pem";
    const PRIVATE_FILENAME: &str = "cert_private.pem";

    let existing_public_certificate = get_server_file_string(PUBLIC_FILENAME).await.unwrap_or(None);
    let existing_private_certificate = get_server_file_string(PRIVATE_FILENAME).await.unwrap_or(None);

    if existing_private_certificate.is_some() && existing_public_certificate.is_some() {
        log_info(LogServiceType::Register, format!("Existing certificate, cheking validity"));

        let public = existing_public_certificate.unwrap();
        let res = parse_x509_pem(public.as_bytes()).unwrap();
        let res_x509 = parse_x509_certificate(&res.1.contents).unwrap();
        log_info(LogServiceType::Register, format!("certificate validity: {:?}",res_x509.1.validity.not_after));

        let expiry: &ASN1Time = &res_x509.1.validity.not_after;
        let utc_time: DateTime<Utc> = Utc::now() + Duration::days(-5);

        let expiry_date: DateTime<Utc> = DateTime::<Utc>::from_timestamp(expiry.timestamp(), 0).expect("invalid timestamp");
         if expiry_date > utc_time { 
            log_info(LogServiceType::Register, format!("Certificate valid"));
            return Ok((get_server_file_path(PUBLIC_FILENAME).await?,get_server_file_path(PRIVATE_FILENAME).await?));
         } else {
            log_info(LogServiceType::Register, format!("Certificate expired"));
         }
    }
    log_info(LogServiceType::Register, format!("No certificates found, requesting new one"));

    let (account, _) = {
        if let Some(existing_credentials) = get_server_file_string(ACCOUNT_FILENAME).await? {
            let credentials: AccountCredentials =  serde_json::from_str(&existing_credentials).unwrap();
            let account: Account = Account::from_credentials(credentials).await.unwrap();
            let credentials: AccountCredentials =  serde_json::from_str(&existing_credentials).unwrap();
            (account, credentials)
        } else {
            log_info(LogServiceType::Register, format!("Create new ACME accounts"));

            let (account, credentials) = Account::create(
                &NewAccount {
                    contact: &[],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                LetsEncrypt::Production.url(),
                None,
            )
            .await.or_else(|_| Err(Error::ServerMalformatedConfigFile))?;

            let serialized_credentials = serde_json::to_string(&credentials).or_else(|_| Err(Error::ServerFileNotFound))?;

            let _ = write_server_file("letsencrypt_account.json", serialized_credentials.as_bytes()).await?;
            (account, credentials)
        }
    };



    let identifier = Identifier::Dns(domain.to_string());
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &[identifier],
        })
        .await
        .unwrap();

    //let state = order.state();
    //println!("order state: {:#?}", state);
    //assert!(matches!(state.status, OrderStatus::Pending));

    let authorizations = order.authorizations().await.unwrap();
    let mut challenges = Vec::with_capacity(authorizations.len());
    for authz in &authorizations {
        //println!("{:?}", authz);
        //match authz.status {
        //    AuthorizationStatus::Pending => {}
        //    AuthorizationStatus::Valid => continue,
        //    _ => todo!(),
        //}

        let challenge = authz
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Dns01)
            .ok_or_else(|| Error::LoginFail)?;

        let Identifier::Dns(identifier) = &authz.identifier;

        log_info(LogServiceType::Register, format!(
            "_acme-challenge.{} IN TXT {}",
            identifier,
            order.key_authorization(challenge).dns_value()
        ));

        let duck_url = format!("https://www.duckdns.org/update?domains={}&token={}&txt={}&verbose=true", domain.replace(".duckdns.org", ""), duck_dns, order.key_authorization(challenge).dns_value());
     
        let _ = reqwest::get(duck_url)
            .await.or_else(|_| Err(Error::Error { message: "Unable to update duckdns".to_string() }))?
            .text()
            .await
            .or_else(|_| Err(Error::Error { message: "Unable to read duckdns response".to_string() }))?;
        
/* 
        println!("Please set the following DNS record then press the Return key:");
        println!(
            "_acme-challenge.{} IN TXT {}",
            identifier,
            order.key_authorization(challenge).dns_value()
        );
        io::stdin().read_line(&mut String::new()).unwrap();*/

        challenges.push((identifier, &challenge.url));
    }

    for (_, url) in &challenges {
        order.set_challenge_ready(url).await.unwrap();
    }

    // Exponentially back off until the order becomes ready or invalid.

    let mut tries = 1u8;
    let mut delay = tokio::time::Duration::from_millis(250);
    loop {
        sleep(delay).await;
        let state = order.refresh().await.unwrap();
        if let OrderStatus::Ready | OrderStatus::Invalid = state.status {
            //println!("order state: {:#?}", state);
            break;
        }

        delay *= 2;
        tries += 1;
        match tries < 10 {
            true => log_info(LogServiceType::Register, format!("order is not ready, waiting {:?}", tries)),
            false => {
                //println!("order is not ready: {:#?}", state);
                return Err(Error::Error { message: "order is not ready".to_string()});
            }
        }
    }

    let state = order.state();
    if state.status != OrderStatus::Ready {
        return Err(Error::Error { message: "unexpected order status:".to_string() });
    }
                
        
    let mut names = Vec::with_capacity(challenges.len());
    for (identifier, _) in &challenges {
        names.push(identifier.to_owned().to_string());
    }

    // If the order is ready, we can provision the certificate.
    // Use the rcgen library to create a Certificate Signing Request.

    let mut params = CertificateParams::new(names.clone());
    params.distinguished_name = DistinguishedName::new();
    let cert = Certificate::from_params(params).unwrap();
    let csr = cert.serialize_request_der().unwrap();

    // Finalize the order and print certificate chain, private key and account credentials.

    order.finalize(&csr).await.unwrap();
    let cert_chain_pem = loop {
        match order.certificate().await.unwrap() {
            Some(cert_chain_pem) => break cert_chain_pem,
            None => sleep(tokio::time::Duration::from_secs(1)).await,
        }
    };

    //println!("certficate chain:\n\n{}", cert_chain_pem);
    //println!("private key:\n\n{}", cert.serialize_private_key_pem());

    let _ = write_server_file("cert_chain.pem", cert_chain_pem.as_bytes()).await?;
    let _ = write_server_file("cert_private.pem", cert.serialize_private_key_pem().as_bytes()).await?;

    log_info(LogServiceType::Register, format!("Certificates created and saved"));

    Ok((get_server_file_path(PUBLIC_FILENAME).await?,get_server_file_path(PRIVATE_FILENAME).await?))
    //Ok((cert_chain_pem, cert.serialize_private_key_pem()))
}

/* 
#[cfg(test)]
mod tests {
    use serial_test::serial;
    use super::*;

    #[tokio::test]
    async fn test_letsencrypt() {
        //certifacte().await;
    }
}*/