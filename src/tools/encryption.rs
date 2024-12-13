use std::{pin::Pin, task::{Context, Poll}};

use aes::{cipher::{block_padding::{Padding, Pkcs7}, BlockDecryptMut, BlockEncryptMut, KeyIvInit}, Aes256, Aes256Dec, Block};
use extism::ToBytes;
use hex_literal::hex;
use pbkdf2::{pbkdf2_hmac, pbkdf2_hmac_array};
use rand::RngCore;
use sha1::Sha1;
use tokio::{fs::{File, OpenOptions}, io::{AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter}};

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;
use futures::Stream;
use x509_parser::nom::bytes::complete;

use crate::error::{RsError, RsResult};

static salt_file: [u8; 24] = hex!("7b9ef4f7aeb46f6d9a6f4f34dfadf471bf7addfef4ddbf37");
static salt_text: [u8; 24] = hex!("6b5db4f7aeb46f7d9c71ad34dfadf471bf7addfef7d1be78");

pub fn ceil_to_multiple_of_16(value: usize) -> usize {
    let ceil = (value + 15) & !15;
    let rest = if value % 16 == 0 { 16 } else { 0 };
    ceil + rest
}


pub fn estimated_encrypted_size(unencrypted_file_size: u64, unencrypted_thumb_size: u64, unencrypted_infos_size: u64) -> u64 {
            //16 Bytes to store IV
        //4 to store encrypted thumb size = T (can be 0)
        //4 to store encrypted Info size = I (can be 0)
        //32 to store thumb mimetype
        //256 to store file mimetype
        //T Bytes for the encrypted thumb
        //I Bytes for the encrypted info
        let thumb_size = if unencrypted_thumb_size == 0 { 0 } else { ceil_to_multiple_of_16(unencrypted_thumb_size as usize) as u64 };
        println!("thumb_size {thumb_size}: {unencrypted_thumb_size}");
        let infos_size = if unencrypted_infos_size == 0 { 0 } else { ceil_to_multiple_of_16(unencrypted_infos_size as usize) as u64 };
        println!("infos_size {infos_size}: {unencrypted_infos_size}");
        let file_size = if unencrypted_file_size == 0 { 0 } else { ceil_to_multiple_of_16(unencrypted_file_size as usize) as u64 };
        println!("file {file_size}: {unencrypted_file_size}");
        16 + 4 + 4 + 32 + 256 + thumb_size +  infos_size + file_size
}

pub fn string_to_fixed_bytes(input: &str, size: usize) -> Vec<u8> {
    let mut bytes = vec![b' '; size]; // Initialize with space characters
    let input_bytes = input.as_bytes();
    let copy_len = input_bytes.len().min(size);
    
    bytes[..copy_len].copy_from_slice(&input_bytes[..copy_len]);
    
    bytes
}

pub struct AesTokioEncryptStream<W: AsyncWrite + Unpin> {
    writer: W,
    encryptor: Aes256CbcEnc,
    buffer: Vec<u8>,
    block_size: usize,

    iv: Vec<u8>,
    header_written: bool,
    file_mime: String,
    thumb: Vec<u8>,
    thumb_mime: String,

    infos: Vec<u8>,
    
}

impl<W: AsyncWrite + Unpin> AesTokioEncryptStream<W> {
    pub fn new(writer: W, key: &[u8], iv: &[u8], file_mime: Option<String>, thumb: Option<(&[u8], String)>, infos: Option<String>) -> RsResult<Self> {
        if key.len() != 32 || iv.len() != 16 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput, 
                "Key must be 256 bytes and IV must be 16 bytes"
            ).into());
        }

        let encryptor = Aes256CbcEnc::new(key.into(), iv.into());        
        let (thumb, thumb_mime) = thumb.unwrap_or((&[], "".to_string()));
        let encthumb = if thumb.len() > 0 {
            encrypt(thumb, key, iv)?
        } else {
            vec![0u8; 0]
        };
        let encinfos = if let Some(infos) = infos {
            let infos_bytes = infos.as_bytes();
            encrypt(&infos_bytes, key, iv)?
        } else {
            vec![0u8; 0]
        };
        Ok(Self {
            writer,
            encryptor,
            buffer: Vec::new(),
            block_size: 16,

            iv: iv.into(),
            header_written: false,
            file_mime: file_mime.unwrap_or("".to_string()),

            thumb: encthumb,
            thumb_mime,
            
            infos: encinfos,
            
        })
    }
    
    

    pub async fn write_encrypted(&mut self, data: &[u8]) -> RsResult<usize> {
        //16 Bytes to store IV
        //4 to store encrypted thumb size = T (can be 0)
        //4 to store encrypted Info size = I (can be 0)
        //32 to store thumb mimetype
        //256 to store file mimetype
        //T Bytes for the encrypted thumb
        //I Bytes for the encrypted info
        if !self.header_written {
            //IV
            self.writer.write_all(self.iv.as_slice()).await?;
        
            //thumb size
            self.writer.write_all(&(self.thumb.len() as u32).to_be_bytes()).await?;

            //info size
            self.writer.write_all(&(self.infos.len() as u32).to_be_bytes()).await?;

            //thumb mime
            self.writer.write_all(&string_to_fixed_bytes(&self.thumb_mime, 32)).await?;

            //file mime
            self.writer.write_all(&string_to_fixed_bytes(&self.file_mime, 256)).await?;

            //enc thumb
            self.writer.write_all(&self.thumb).await?;

            //enc infos
            self.writer.write_all(&self.infos).await?;

            self.header_written = true;
            //println!("Written headers");
        }



        self.buffer.extend_from_slice(data);

        // Determine how many complete blocks we have
        let complete_blocks = self.buffer.len() / self.block_size;
        if complete_blocks == 0 {
            return Ok(data.len());
        }
        let complete_blocks = complete_blocks - 1;
        let blocks_end = complete_blocks * self.block_size;

        // Split into blocks to decrypt and leftover
        let (blocks, rest) = self.buffer.split_at(blocks_end);
        // Check if it's last block and send it to finalize for padding

        // Encrypt multiple blocks at once
        let mut decrypted_blocks = Vec::with_capacity(blocks_end);
        for chunk in blocks.chunks(self.block_size) {
            let mut block_array: [u8; 16] = chunk.try_into().map_err(|e| RsError::CryptError("Unable to convert chuck to block_array".to_string()))?;
            let mut outblock = Block::default();
            self.encryptor.encrypt_block_b2b_mut(&mut block_array.into(), &mut outblock);
            decrypted_blocks.extend_from_slice(&outblock);
        }

        // Write all decrypted blocks in one go
        self.writer.write_all(&decrypted_blocks).await?;

        // Keep only the leftover bytes in the buffer
        self.buffer = rest.to_vec();

        Ok(data.len())
    }

    pub async fn finalize(mut self) -> RsResult<()> {
        //println!("finalize: {}", self.buffer.len());
        if !self.buffer.is_empty() {

            let mut buf = vec![0; ceil_to_multiple_of_16(self.buffer.len())];
            let pt = self.encryptor.encrypt_padded_b2b_mut::<Pkcs7>(&self.buffer, &mut buf)?;

            
            self.writer.write_all(&pt).await?;
        }
        self.writer.flush().await?;
        Ok(())
    }
}

impl<W: AsyncWrite + Unpin> Stream for AesTokioEncryptStream<W> {
    type Item = std::io::Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Implementation would depend on specific streaming requirements
        Poll::Ready(None)
    }
}




//======================================================
pub struct AesTokioDecryptStream<W: AsyncWrite + Unpin> {
    writer: W,
    decryptor: Option<Aes256CbcDec>,
    buffer: Vec<u8>,
    key: Vec<u8>,
    block_size: usize,

    thumb_size: Option<u32>,
    thumb_passed: bool,
    infos_size: Option<u32>,
    infos_passed: bool,

    thumb_mime_passed: bool,
    file_mime_passed: bool,
}

impl<W: AsyncWrite + Unpin> AesTokioDecryptStream<W> {
    pub fn new(writer: W, key: &[u8], iv: Option<&[u8]>) -> Result<Self, std::io::Error> {
        if key.len() != 32 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput, 
                "Key must be 32 bytes"
            ));
        }

        
        Ok(Self {
            writer,
            decryptor: if let Some(iv) = iv {
                Some(Aes256CbcDec::new(key.into(), iv.into()))
            } else {
                None
            },
            buffer: Vec::new(),
            key: key.into(),
            block_size: 16,

            thumb_size: None,
            thumb_passed: false,
            infos_size: None,
            infos_passed: false,

            thumb_mime_passed: false,
            file_mime_passed: false,
            
        })
    }

    pub async fn write_decrypted(&mut self, data: &[u8]) -> RsResult<usize> {
        self.buffer.extend_from_slice(data);

        //16 Bytes to store IV
        //4 to store encrypted thumb size = T (can be 0)
        //4 to store encrypted Info size = I (can be 0)
        //32 to store thumb mimetype
        //256 to store file mimetype
        //T Bytes for the encrypted thumb
        //I Bytes for the encrypted info
        if let Some(mut decryptor) = self.decryptor.as_mut() {
          
            if self.thumb_size.is_none() {
                if self.buffer.len() < 4 {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(4);
                self.thumb_size = Some(u32::from_be_bytes(block[0..4].try_into().map_err(|e| RsError::CryptError("Unable to convert chuck to block_array".to_string()))?));
                println!("thumb size: {:?}", self.thumb_size);
                self.buffer = rest.to_vec();
            }
            if self.infos_size.is_none() {
                if self.buffer.len() < 4 {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(4);
                self.infos_size = Some(u32::from_be_bytes(block[0..4].try_into().map_err(|e| RsError::CryptError("Unable to convert chuck to block_array".to_string()))?));
                println!("info size: {:?}", self.infos_size);
                self.buffer = rest.to_vec();
            }

            if !self.thumb_mime_passed {
                if self.buffer.len() < 32 {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(32);
                self.buffer = rest.to_vec();
                self.thumb_mime_passed = true;
            }
            if !self.file_mime_passed {
                if self.buffer.len() < 256 {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(256);
                let file_mime = String::from_utf8(block.to_vec()).unwrap_or("unable to decrypt".to_string());
                println!("filemime: {}", file_mime);
                self.buffer = rest.to_vec();
                self.file_mime_passed = true;
            }


            let thumb_size = self.thumb_size.unwrap_or(0);
            if !self.thumb_passed && thumb_size > 0 {
                if self.buffer.len() < thumb_size as usize {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(thumb_size as usize);
                println!("passed thumb of {}", block.len());
                self.buffer = rest.to_vec();
                self.thumb_passed = true;
            }
            let infos_size = self.infos_size.unwrap_or(0);
            if !self.infos_passed && infos_size > 0 {
                if self.buffer.len() < infos_size as usize {
                    return Ok(data.len());
                }
                let (block, rest) = self.buffer.split_at(infos_size as usize);
                println!("passed infos of {}", block.len());
                self.buffer = rest.to_vec();
                self.infos_passed = true;
            }


            //put the last 16 bits in a end_buffer in case it's the end of the file
            //let (block, rest) = self.buffer.split_at();

            // Determine how many complete blocks we have (substracting a block here in case it's the last one with padding)
            let complete_blocks = (self.buffer.len() / self.block_size) - 1;
            let blocks_end = complete_blocks * self.block_size;

            // Split into blocks to decrypt and leftover
            let (blocks, rest) = self.buffer.split_at(blocks_end);

            // Decrypt multiple blocks at once
            let mut decrypted_blocks = Vec::with_capacity(blocks_end);
            for chunk in blocks.chunks(self.block_size) {
                let mut block_array: [u8; 16] = chunk.try_into().map_err(|e| RsError::CryptError("Unable to convert chuck to block_array".to_string()))?;
                //println!("encrypted {:?}", block_array);
                let mut outblock = Block::default();
                decryptor.decrypt_block_b2b_mut(&mut block_array.into(), &mut outblock);
                decrypted_blocks.extend_from_slice(&outblock);
                //print!("outblock: {:?}", decrypted_blocks);

                //return Err(std::io::Error::other("STOP".to_string()));

            }

            // Write all decrypted blocks in one go
            self.writer.write_all(&decrypted_blocks).await?;


            // Keep only the leftover bytes in the buffer
            self.buffer = rest.to_vec();

        } else if self.buffer.len() >= 16 {
            let (blocks, rest) = self.buffer.split_at(16);
            println!("iv: {:?}", blocks);



            self.decryptor = Some(Aes256CbcDec::new(self.key.as_slice().into(), blocks.into()));
            
            // init again the buffer buffer with the remaining
            self.buffer = rest.to_vec();

        }

        Ok(data.len())

    }

    pub async fn finalize(mut self) -> RsResult<()> {
        if let Some(mut decryptor) = self.decryptor {
            if !self.buffer.is_empty() {
                let mut padded_block: [u8; 16] = self.buffer.try_into().map_err(|e| RsError::CryptError("Unable to convert chuck to block_array".to_string()))?;


                let mut outblock = Block::default();
                decryptor.decrypt_block_b2b_mut(&mut padded_block.into(), &mut outblock);

                let padded_block = outblock.as_slice();

                // Remove PKCS7 padding
                let pad_length = padded_block[padded_block.len() - 1] as usize;
                let unpadded = &padded_block[..padded_block.len() - pad_length];
                
                self.writer.write_all(unpadded).await?;
            }
            self.writer.flush().await?;
        }
        Ok(())
    }
}

impl<W: AsyncWrite + Unpin> Stream for AesTokioDecryptStream<W> {
    type Item = std::io::Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}




//======================================================






pub fn derive_key(password: String) -> [u8; 32] {
    let password = password.into_bytes();
    //let salt = b"salt";
    // number of iterations
    let n = 1000;

    let mut key1 = [0u8; 32];
    pbkdf2_hmac::<Sha1>(password.as_slice(), &salt_file, n, &mut key1);
    //println!("{:?}", key1);
    
    key1
}

fn encrypt(data: &[u8], key: &[u8], iv: &[u8]) -> RsResult<Vec<u8>> {
    if data.len() == 0 {
        return Ok(data.to_vec());
    }
    let data_len = data.len();
    let mut buf = vec![0u8; ceil_to_multiple_of_16(data_len)];
    
    buf[..data_len].copy_from_slice(&data);
    let ct = Aes256CbcEnc::new(key.into(), iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, data_len)?;

    Ok((ct.to_vec()))
}

fn decrypt(mut data: &mut [u8], key: &[u8], iv: &[u8]) -> RsResult<Vec<u8>> {

    let decrypted = Aes256CbcDec::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut data)
        .map_err(|e| RsError::CryptError("Unable to convert chuck todecrypt data buffer".to_string()))?;

    Ok(decrypted.to_vec())
}

pub fn random_iv() -> [u8; 16] {
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);
    iv
}

pub async fn encrypt_file(
    input_path: &str, 
    output_path: &str, 
    key: &[u8], 
    iv: &[u8]
) -> RsResult<()> {
    // Open input file
    let input_file = File::open(input_path).await?;
    let mut reader = BufReader::new(input_file);

    // Create output file
    let output_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_path)
        .await?;
    let writer = BufWriter::new(output_file);

    // Create encryption stream
    let mut encrypt_stream = AesTokioEncryptStream::new(writer, key, iv, None, Some((vec![0, 1, 226, 64, 100, 200, 50, 75, 0, 0, 0, 0].as_slice(), "image/jpeg".to_string())), None)?;

    // Read and encrypt in chunks
    let mut buffer = vec![0; 1024];
    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        encrypt_stream.write_encrypted(&buffer[..bytes_read]).await?;
    }

    // Finalize encryption
    encrypt_stream.finalize().await?;

    Ok(())
}

pub async fn decrypt_file(
    input_path: &str, 
    output_path: &str, 
    key: &[u8], 
    iv: Option<&[u8]>
) -> RsResult<()> {
    let input_file = File::open(input_path).await?;
    let mut reader = BufReader::new(input_file);

    let output_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_path)
        .await?;
    let writer = BufWriter::new(output_file);

    let mut decrypt_stream = AesTokioDecryptStream::new(writer, key, iv)?;

    let mut buffer = vec![0; 1024];
    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        decrypt_stream.write_decrypted(&buffer[..bytes_read]).await?;
    }

    decrypt_stream.finalize().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encrypt() {
        let key = derive_key("test".to_string());
        let iv = random_iv();
        //c:\\Devs\\test.png
        ///Users/arnaudjezequel/Documents/video.mp4
       encrypt_file("/Users/arnaudjezequel/Documents/video.mp4", "/Users/arnaudjezequel/Documents/video.enc", &key, &iv).await.unwrap();
       decrypt_file("/Users/arnaudjezequel/Documents/video.enc", "/Users/arnaudjezequel/Documents/video.enc.mp4", &key, None).await.unwrap();
       //test_file("/Users/arnaudjezequel/Documents/video.mp4","/Users/arnaudjezequel/Documents/video.mp4.enc", "/Users/arnaudjezequel/Documents/video.mp4.dec.mp4", &key, &iv).await.unwrap();
       
       //decrypt_file("/Users/arnaudjezequel/Downloads/U-AqTolcHF-H0vBi8mtpHQ", "/Users/arnaudjezequel/Downloads/U-AqTolcHF-H0vBi8mtpHQ.heic", &key, None).await.unwrap();

    }


    
}