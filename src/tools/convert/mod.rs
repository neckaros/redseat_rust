use crate::error::{RsError, RsResult};
use ort::Utf8Data;
use rs_torrent_magnet::magnet_from_torrent;
use tokio::io::{AsyncRead, AsyncReadExt};
pub mod magnet;

pub struct ConvertFileSource<T: Sized + AsyncRead + Send + Unpin > {
    pub mime: String,
    pub reader: T
}

pub async fn convert_from_to<T: Sized + AsyncRead + Send + Unpin >(mut source: ConvertFileSource<T>, target: &str) -> RsResult<Vec<u8>> {
    if source.mime == "application/x-bittorrent" && target == "text/x-uri" {
        let mut buffer = Vec::new();
        source.reader.read_to_end(&mut buffer).await?;
        let result = magnet_from_torrent(buffer).map_err(|_| RsError::ConversionFailed(source.mime, target.to_string()))?;
        Ok(result.as_utf8_bytes().to_vec())
    } else {
        Err(RsError::CouldNotFindConvertor(source.mime, target.to_string()))
    }
    


}