use std::{pin::Pin, task::{Context, Poll}};

use aes::cipher::{block_padding::{Padding, Pkcs7}, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hex_literal::hex;
use pbkdf2::{pbkdf2_hmac, pbkdf2_hmac_array};
use rand::RngCore;
use sha1::Sha1;
use tokio::{fs::{File, OpenOptions}, io::{AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter}};
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
use futures::Stream;

use crate::error::RsResult;


pub struct AesTokioEncryptStream<W: AsyncWrite + Unpin> {
    writer: W,
    encryptor: Aes128CbcEnc,
    buffer: Vec<u8>,
    block_size: usize,
}

impl<W: AsyncWrite + Unpin> AesTokioEncryptStream<W> {
    pub fn new(writer: W, key: &[u8], iv: &[u8]) -> Result<Self, std::io::Error> {
        if key.len() != 16 || iv.len() != 16 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput, 
                "Key and IV must be 16 bytes"
            ));
        }

        let encryptor = Aes128CbcEnc::new(key.into(), iv.into());
        
        Ok(Self {
            writer,
            encryptor,
            buffer: Vec::new(),
            block_size: 16,
        })
    }

    pub async fn write_encrypted(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(data);

        let complete_blocks = self.buffer.len() / self.block_size;
        for _ in 0..complete_blocks {
            let (block, rest) = self.buffer.split_at(self.block_size);
            let mut encrypted_block: [u8; 16] = block.try_into().unwrap();
            self.encryptor.encrypt_block_mut(&mut encrypted_block.into());
            self.writer.write_all(&encrypted_block).await?;
            self.buffer = rest.to_vec();
        }

        Ok(data.len())
    }

    pub async fn finalize(mut self) -> std::io::Result<()> {
        if !self.buffer.is_empty() {
            // Manually pad the buffer to 16 bytes
            let mut padded_block = [0u8; 16];
            let pad_length = self.block_size - (self.buffer.len() % self.block_size);
            padded_block[..self.buffer.len()].copy_from_slice(&self.buffer);
            
            // PKCS7 padding
            for i in self.buffer.len()..16 {
                padded_block[i] = pad_length as u8;
            }

            self.encryptor.encrypt_block_mut(&mut padded_block.into());
            self.writer.write_all(&padded_block).await?;
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
    decryptor: Aes128CbcDec,
    buffer: Vec<u8>,
    block_size: usize,
}

impl<W: AsyncWrite + Unpin> AesTokioDecryptStream<W> {
    pub fn new(writer: W, key: &[u8], iv: &[u8]) -> Result<Self, std::io::Error> {
        if key.len() != 16 || iv.len() != 16 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput, 
                "Key and IV must be 16 bytes"
            ));
        }

        let decryptor = Aes128CbcDec::new(key.into(), iv.into());
        
        Ok(Self {
            writer,
            decryptor,
            buffer: Vec::new(),
            block_size: 16,
        })
    }

    pub async fn write_decrypted(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(data);

        let complete_blocks = self.buffer.len() / self.block_size;
        for _ in 0..complete_blocks {
            let (block, rest) = self.buffer.split_at(self.block_size);
            let mut block_array: [u8; 16] = block.try_into().unwrap();
            self.decryptor.decrypt_block_mut(&mut block_array.into());
            self.writer.write_all(&block_array).await?;
            self.buffer = rest.to_vec();
        }

        Ok(data.len())
    }

    pub async fn finalize(mut self) -> std::io::Result<()> {
        if !self.buffer.is_empty() {
            let mut padded_block = [0u8; 16];
            padded_block[..self.buffer.len()].copy_from_slice(&self.buffer);

            self.decryptor.decrypt_block_mut(&mut padded_block.into());

            // Remove PKCS7 padding
            let pad_length = padded_block[padded_block.len() - 1] as usize;
            let unpadded = &padded_block[..padded_block.len() - pad_length];
            
            self.writer.write_all(unpadded).await?;
        }
        self.writer.flush().await?;
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






fn derive_key() -> [u8; 16] {
    let password = b"password";
    let salt = b"salt";
    // number of iterations
    let n = 1000;
    // Expected value of generated key
    let expected = hex!("6e88be8bad7eae9d9e10aa061224034f");

    let mut key1 = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password, salt, n, &mut key1);
    println!("{:x?}", key1);
    let hex_string = hex::encode(key1);
    assert_eq!(key1, expected);
    key1
}
fn test() {
    let key = [0x42; 16];
    let iv = [0x24; 16];
    let plaintext = *b"hello world! this is my plaintext.";
    let ciphertext = hex!(
        "c7fe247ef97b21f07cbdd26cb5d346bf"
        "d27867cb00d9486723e159978fb9a5f9"
        "14cfb228a710de4171e396e7b6cf859e"
    );

    // encrypt/decrypt in-place
    // buffer must be big enough for padded plaintext
    let mut buf = [0u8; 48];
    let pt_len = plaintext.len();
    buf[..pt_len].copy_from_slice(&plaintext);
    let ct = Aes128CbcEnc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, pt_len)
        .unwrap();
    assert_eq!(ct, &ciphertext[..]);

    let pt = Aes128CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .unwrap();
    assert_eq!(pt, &plaintext);

    // encrypt/decrypt from buffer to buffer
    let mut buf = [0u8; 48];
    let ct = Aes128CbcEnc::new(&key.into(), &iv.into())
        .encrypt_padded_b2b_mut::<Pkcs7>(&plaintext, &mut buf)
        .unwrap();
    assert_eq!(ct, &ciphertext[..]);

    let mut buf = [0u8; 48];
    let pt = Aes128CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_b2b_mut::<Pkcs7>(&ct, &mut buf)
        .unwrap();
    assert_eq!(pt, &plaintext);
}

fn random_iv() -> [u8; 16] {
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
    let mut encrypt_stream = AesTokioEncryptStream::new(writer, key, iv)?;

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
    iv: &[u8]
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
        let key = derive_key();
        let iv = hex!("6e88be8bad7eae9d9e10aa061224034f");
        encrypt_file("c:\\Devs\\test.png", "c:\\Devs\\test.enc", &key, &iv).await.unwrap();
        
        decrypt_file("c:\\Devs\\test.enc", "c:\\Devs\\test.dec.png", &key, &iv).await.unwrap();
       test();
    }


    
}