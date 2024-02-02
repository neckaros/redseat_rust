use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::error::Result;
use crate::server::{has_server_file, write_server_file};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    name: String,
    aud: String,
    exp: u64,
}

pub fn verify(token: &str, server: &str) -> Result<Claims> {
    let public_key = DecodingKey::from_rsa_pem(include_bytes!("pubkey.pem")).expect("Unable to find publickey.pem");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[server]);
    let token_data = match decode::<Claims>(&token, &public_key, &validation) {
        Ok(token) => token,
        Err(e) => match e.kind() {
            ErrorKind::InvalidToken => return Err(Error::AuthFailTokenWrongFormat),
            ErrorKind::InvalidSignature => return Err(Error::AuthFailInvalidToken),
            ErrorKind::MissingRequiredClaim(_) => return Err(Error::AuthFailInvalidToken),
            ErrorKind::ExpiredSignature => return Err(Error::AuthFailExpiredToken),
            ErrorKind::InvalidIssuer => return Err(Error::AuthFailInvalidToken),
            ErrorKind::InvalidAudience => return Err(Error::AuthFailNotForThisServer),
            ErrorKind::InvalidSubject => return Err(Error::AuthFailInvalidToken),
            _ => return Err(Error::AuthFailInvalidToken),
        },
    };
    Ok(token_data.claims)
}

pub async fn get_or_init_keys() -> Result<()> {

    if has_server_file("pubkey.pem").await && has_server_file("private.pem").await {
        return  Ok(());
    }

    let mut rng = rand::thread_rng();
    let bits = 2048;
    let priv_key = RsaPrivateKey::new(&mut rng, bits).expect("failed to generate a key");
    let pub_key = RsaPublicKey::from(&priv_key);

    let pubkeystring = pub_key.to_public_key_pem(rsa::pkcs8::LineEnding::CRLF).unwrap();

    let prvkeystring = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::CRLF).unwrap();

    write_server_file("pubkey.pem", pubkeystring.as_bytes()).await?;
    write_server_file("private.pem", prvkeystring.as_bytes()).await?;

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verifytoken() {
        let claims = verify("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJURVNUVVNFUiIsIm5hbWUiOiJuZWNrYXJvcyIsImF1ZCI6IkZPUlRFU1RPTkxZIiwiaWF0IjoxNzA2NTM5MDMyLCJleHAiOjk3MDY1NjA2MzJ9.kpc4EKQuosxlaouTOyp-bWsTHQLGUh3Om1uFX95P5loskwSzuDPw87KfGENr1vqMEiQ08S5J-6fLNQUGVn35Jq6HFzRVPuu3MThZHjh3DuY1kkXBJLfnRgMSQYC5dKosfkXSduQ82N_lN6JZSKWvbxSqVuEpRPA1ws854iUlasE1lzqMQXT3goT0p8FNSg7IqkUl39MEqR350nCNwP72igI9A8V71K-dzbf_xqvKauN1xomxQbHV-OWZ2gzbcRMwAIbTq150WKarTLQxLqKqG-Cm_dfABmaHoefXvW7BTNW-OAwy9S0zOhPZqkAF6u55k2tWdDbi5hZ6WsbcGvh6oA", "FORTESTONLY").unwrap();
        assert_eq!(claims.name, "neckaros");
        assert_eq!(claims.sub, "TESTUSER");
        assert_eq!(claims.exp, 9706560632u64);
    }

    #[test]
    fn expired_token() {
        let _error = verify("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJURVNUVVNFUiIsIm5hbWUiOiJuZWNrYXJvcyIsImF1ZCI6IkZPUlRFU1RPTkxZIiwiaWF0IjoxNzA2NTM5MDMyLCJleHAiOjE2MDY1NjA2MzJ9.GcgyBNU_4endKOmRWEzNdG1znRChN9D4iz_BGVbdQxy_5uMtF2hSBWGdBZ2iG--YOFnbpwNalhIJkFHDWnOMQm7h8pCbitWBmYy693nOYp7H7CoMJ3PKFq8uPhcfkqfXNsH4V0TI4Y1iGkdgf_35FYKmOFSW7_xGMyyh0OHaWSpiIeggAd5tb7laj1wyM2Vb15KpMVuL-6xCkxBBmBMQEjM00Cl24_yfvrDs9sZsvgyMIZU4IaSDqjmrvuQogSuIIzrNxhyLI_jNQffT3OaQ3qRoTAULrgiJGP8PfZADER1099uabCv41U9ZJiYbrK5tZUxknVEkMwJsDWtvL3P1kg", "FORTESTONLY")        
        .unwrap_err();

        //assert_eq!(error, Error::AuthFailExpiredToken);
    }

    #[test]
    fn forotherserver_token() {
        let _error = verify("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJURVNUVVNFUiIsIm5hbWUiOiJuZWNrYXJvcyIsImF1ZCI6IkZPUlRFU1RPTkxZIiwiaWF0IjoxNzA2NTM5MDMyLCJleHAiOjk3MDY1NjA2MzJ9.kpc4EKQuosxlaouTOyp-bWsTHQLGUh3Om1uFX95P5loskwSzuDPw87KfGENr1vqMEiQ08S5J-6fLNQUGVn35Jq6HFzRVPuu3MThZHjh3DuY1kkXBJLfnRgMSQYC5dKosfkXSduQ82N_lN6JZSKWvbxSqVuEpRPA1ws854iUlasE1lzqMQXT3goT0p8FNSg7IqkUl39MEqR350nCNwP72igI9A8V71K-dzbf_xqvKauN1xomxQbHV-OWZ2gzbcRMwAIbTq150WKarTLQxLqKqG-Cm_dfABmaHoefXvW7BTNW-OAwy9S0zOhPZqkAF6u55k2tWdDbi5hZ6WsbcGvh6oA", "SPECIFICSERVER")        
        .unwrap_err();

        //assert_eq!(error, Error::AuthFailNotForThisServer);
    }

}