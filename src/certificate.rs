
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use rcgen::{Certificate, CertificateParams, DistinguishedName};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use reqwest;

use instant_acme::{
    Account, AccountCredentials, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus
};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem, time::ASN1Time};

use crate::{error::Error, server::{get_config, get_server_file_path, get_server_file_string, write_server_file}, tools::log::{log_info, LogServiceType}, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TxtRecord {
    pub txt: Vec<String>,
}


pub async fn dns_certify() -> Result<(PathBuf, PathBuf)> {
    let config = get_config().await;

    let id = config.id.ok_or(crate::Error::ServerNoServerId)?;
    let token = config.token.ok_or(crate::Error::ServerNotYetRegistered)?;



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
                //LetsEncrypt::Production.url(),
                None,
            )
            .await.map_err(|_| Error::ServerMalformatedConfigFile)?;

            let serialized_credentials = serde_json::to_string(&credentials).or_else(|_| Err(Error::ServerFileNotFound))?;

            write_server_file("letsencrypt_account.json", serialized_credentials.as_bytes()).await?;
            (account, credentials)
        }
    };



    let domain = format!("{}-srv.redseat.cloud", id);
    let subdomain = format!("*.{}-srv.redseat.cloud", id);
    let identifier = Identifier::Dns(domain.clone());
    let identifiersub = Identifier::Dns(subdomain.clone());
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &[identifier, identifiersub],
        })
        .await
        .unwrap();

    //let state = order.state();
    //println!("order state: {:#?}", state);
    //assert!(matches!(state.status, OrderStatus::Pending));

    let authorizations = order.authorizations().await.unwrap();
    let mut challenges = Vec::with_capacity(authorizations.len());
    let mut challenges_txt =  Vec::with_capacity(authorizations.len());
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

        challenges_txt.push(order.key_authorization(challenge).dns_value());
       
        
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

    let request = TxtRecord { txt: challenges_txt};
    let client = reqwest::Client::new();

    let result = client.patch(format!("https://{}/servers/{}/register/txt", config.redseat_home, id))
        .header("Authorization", format!("Token {}", token))
        .json(&request)
        .send()
        .await?;
    let json = result.text().await?;
    log_info(LogServiceType::Register, format!(
        "retour {:?}",
        json
    ));
    
    
    log_info(LogServiceType::Register, format!(
        "Waiting 60 seconds for DNS propagation {} {:?}",
        format!("https://{}/servers/{}/register/txt", config.redseat_home, id),
        request
    ));
    sleep(std::time::Duration::from_secs(60)).await;

    for (_, url) in &challenges {
        order.set_challenge_ready(url).await.map_err(|_| Error::Error(format!("Unable to set challenge ready for {}", url)))?;
    }

    // Exponentially back off until the order becomes ready or invalid.

    let mut tries = 1u8;
    let mut delay = tokio::time::Duration::from_millis(250);
    loop {
        sleep(delay).await;
        let state = order.refresh().await.map_err(|_| Error::Error("Unable to refresh order rstatus".to_string()))?;
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
                return Err(Error::Error("order is not ready".to_string()));
            }
        }
    }

    let state = order.state();
    if state.status != OrderStatus::Ready {
        return Err(Error::Error("unexpected order status:".to_string()));
    }
                
        
    let mut names = Vec::with_capacity(challenges.len());
    /*for (identifier, _) in &challenges {
        names.push(identifier.to_owned().to_string());
    }*/
    names.push(domain);
    names.push(subdomain);

    // If the order is ready, we can provision the certificate.
    // Use the rcgen library to create a Certificate Signing Request.
   
    log_info(LogServiceType::Register, format!(
        "Certificate names {:?}",
        names
    ));  

    let mut params = CertificateParams::new(names.clone());
    params.distinguished_name = DistinguishedName::new();
    let cert = Certificate::from_params(params).map_err(|_| Error::Error(format!("Unable to create certificate from params")))?;
    let csr = cert.serialize_request_der().map_err(|_| Error::Error(format!("Unable to serialiaze certificate")))?;

    // Finalize the order and print certificate chain, private key and account credentials.

    order.finalize(&csr).await.map_err(|e| Error::Error(format!("Unable to finalize CSR {:?}", e)))?;
    let cert_chain_pem: String = loop {
        match order.certificate().await.map_err(|_| Error::Error(format!("Unable to get finale certificate")))? {
            Some(cert_chain_pem) => break cert_chain_pem,
            None => sleep(tokio::time::Duration::from_secs(1)).await,
        }
    };

    //println!("certficate chain:\n\n{}", cert_chain_pem);
    //println!("private key:\n\n{}", cert.serialize_private_key_pem());

    let _ = write_server_file(PUBLIC_FILENAME.clone(), cert_chain_pem.as_bytes()).await?;
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