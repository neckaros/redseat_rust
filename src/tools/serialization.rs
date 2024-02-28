use serde::Serialize;

pub fn optional_serde_to_string<T: Serialize>(serialized: Option<T>) -> serde_json::Result<Option<String>> {
    if let Some(so) = serialized {
        let r = Some(serde_json::to_string(&so)?);
        Ok(r)
    } else {
        Ok(None)
    }
}