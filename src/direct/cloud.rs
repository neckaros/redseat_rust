//! Cloud endpoints for direct HTTPS (`/api/servers/<id>/…`), authenticated with the
//! registration token. Times are Unix milliseconds.

use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::error::{RsError, RsResult};

pub struct CloudClient {
    client: Client,
    base_url: String,
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelInfo {
    pub label: String,
    #[serde(default)]
    pub pending_label: Option<String>,
}

/// `PATCH /api/servers/<id>` body. Each call replaces the stored candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddressReport {
    pub lan: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4: Option<String>,
    pub ipv6: Vec<String>,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateStatus {
    pub label: String,
    #[serde(default)]
    pub pending_label: Option<String>,
    /// The name the next CSR must request.
    pub name: String,
    #[serde(default)]
    pub certificate: Option<IssuedCertificate>,
    #[serde(default)]
    pub csr_needed: bool,
    #[serde(default)]
    pub order: Option<CertificateOrder>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedCertificate {
    pub label: String,
    pub chain: String,
    pub not_before: i64,
    pub not_after: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateOrder {
    pub id: String,
    /// `queued`, `processing` or `failed`.
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub queue: Option<OrderQueue>,
}

impl CertificateOrder {
    pub fn is_failed(&self) -> bool {
        self.status == "failed"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderQueue {
    #[serde(default)]
    pub position: Option<u64>,
    #[serde(default)]
    pub not_before: Option<i64>,
}

/// Result of `POST /api/servers/<id>/certificate`.
#[derive(Debug)]
pub enum CsrOutcome {
    Queued,
    /// An order is already running for this server.
    Processing,
    /// Not due for renewal, weekly limit reached, CSR rejected, or issuance unavailable.
    Refused { status: StatusCode, message: String },
}

#[derive(Deserialize)]
struct ErrorBody {
    message: Option<String>,
}

#[derive(Deserialize)]
struct OrderBody {
    #[serde(rename = "orderId")]
    order_id: Option<String>,
}

impl CloudClient {
    pub fn new(home: &str, server_id: &str, token: &str) -> RsResult<Self> {
        let home = home
            .trim_end_matches('/')
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        Ok(Self {
            client: Client::builder().timeout(Duration::from_secs(30)).build()?,
            base_url: format!("https://{home}/api/servers/{server_id}"),
            token: token.to_string(),
        })
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request.header("Authorization", format!("Token {}", self.token))
    }

    pub async fn label(&self) -> RsResult<LabelInfo> {
        let response = self
            .authorized(self.client.get(format!("{}/label", self.base_url)))
            .send()
            .await?;
        Ok(ok_or_error(response).await?.json().await?)
    }

    /// Starts a label rotation; the new label is `pendingLabel` until its certificate is issued.
    pub async fn rotate_label(&self) -> RsResult<LabelInfo> {
        let response = self
            .authorized(self.client.post(format!("{}/label", self.base_url)))
            .send()
            .await?;
        Ok(ok_or_error(response).await?.json().await?)
    }

    pub async fn report_addresses(&self, report: &AddressReport) -> RsResult<()> {
        let response = self
            .authorized(self.client.patch(&self.base_url))
            .json(report)
            .send()
            .await?;
        ok_or_error(response).await?;
        Ok(())
    }

    pub async fn report_members(&self, users: &[String]) -> RsResult<()> {
        let response = self
            .authorized(self.client.put(format!("{}/members", self.base_url)))
            .json(&serde_json::json!({ "users": users }))
            .send()
            .await?;
        ok_or_error(response).await?;
        Ok(())
    }

    pub async fn certificate_status(&self) -> RsResult<CertificateStatus> {
        let response = self
            .authorized(self.client.get(format!("{}/certificate", self.base_url)))
            .send()
            .await?;
        Ok(ok_or_error(response).await?.json().await?)
    }

    pub async fn request_certificate(&self, csr_pem: &str) -> RsResult<CsrOutcome> {
        let response = self
            .authorized(self.client.post(format!("{}/certificate", self.base_url)))
            .json(&serde_json::json!({ "csr": csr_pem }))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(CsrOutcome::Queued);
        }
        // 409 carries an order id when one is running, a message when not due for renewal.
        if status == StatusCode::CONFLICT
            && serde_json::from_str::<OrderBody>(&body)
                .ok()
                .and_then(|order| order.order_id)
                .is_some()
        {
            return Ok(CsrOutcome::Processing);
        }
        if matches!(
            status,
            StatusCode::CONFLICT
                | StatusCode::TOO_MANY_REQUESTS
                | StatusCode::BAD_REQUEST
                | StatusCode::SERVICE_UNAVAILABLE
        ) {
            return Ok(CsrOutcome::Refused {
                status,
                message: error_message(&body),
            });
        }
        Err(RsError::Error(format!(
            "Certificate request failed ({status}): {}",
            error_message(&body)
        )))
    }
}

async fn ok_or_error(response: Response) -> RsResult<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let url = response.url().path().to_string();
    let body = response.text().await.unwrap_or_default();
    Err(RsError::Error(format!(
        "{url} failed ({status}): {}",
        error_message(&body)
    )))
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|error| error.message)
        .unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_certificate_status() {
        let status: CertificateStatus = serde_json::from_value(serde_json::json!({
            "label": "abc",
            "pendingLabel": "def",
            "name": "*.def.servers.redseat.cloud",
            "certificate": { "label": "abc", "chain": "PEM", "notBefore": 1, "notAfter": 2 },
            "renewal": { "start": 0, "end": 0, "fromAri": true },
            "csrNeeded": true,
            "order": { "id": "o1", "status": "queued", "createdAt": 0, "attempts": 0,
                       "nextAttemptAt": 0, "queue": { "position": 1, "notBefore": 5 } },
            "issuance": { "configured": true, "weeklyBudget": 45, "usedThisWeek": 3 }
        }))
        .unwrap();
        assert_eq!(status.pending_label.as_deref(), Some("def"));
        assert!(status.csr_needed);
        assert_eq!(status.certificate.unwrap().not_before, 1);
        let order = status.order.unwrap();
        assert!(!order.is_failed());
        assert_eq!(order.queue.unwrap().not_before, Some(5));

        let minimal: CertificateStatus = serde_json::from_value(serde_json::json!({
            "label": "abc", "name": "*.abc.servers.redseat.cloud", "csrNeeded": false
        }))
        .unwrap();
        assert!(minimal.certificate.is_none() && minimal.order.is_none());
    }

    #[test]
    fn address_report_omits_missing_ipv4() {
        let report = AddressReport {
            lan: vec!["192.168.1.10".into()],
            ipv4: None,
            ipv6: vec![],
            port: 8080,
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({ "lan": ["192.168.1.10"], "ipv6": [], "port": 8080 })
        );
    }
}
