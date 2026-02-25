use std::{pin::Pin, task::{Context, Poll}};

use aes::{cipher::{block_padding::{Padding, Pkcs7}, BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher, StreamCipherSeek}, Aes256, Aes256Dec, Block};
use extism::ToBytes;
use hex_literal::hex;
use pbkdf2::{pbkdf2_hmac, pbkdf2_hmac_array};
use rand::RngCore;
use sha1::Sha1;
use tokio::{fs::{File, OpenOptions}, io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadBuf}};

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256Ctr = ctr::Ctr128BE<Aes256>;
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
        //println!("thumb_size {thumb_size}: {unencrypted_thumb_size}");
        let infos_size = if unencrypted_infos_size == 0 { 0 } else { ceil_to_multiple_of_16(unencrypted_infos_size as usize) as u64 };
        //println!("infos_size {infos_size}: {unencrypted_infos_size}");
        let file_size = if unencrypted_file_size == 0 { 0 } else { ceil_to_multiple_of_16(unencrypted_file_size as usize) as u64 };
        //println!("file {file_size}: {unencrypted_file_size}");
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

//======================================================
// AES-256-CTR Streaming Encryption/Decryption
// Used for password-protected library file encryption.
// Format: [16-byte nonce][CTR-encrypted data]
// Supports random access (range requests) via counter seeking.
//======================================================

pub const CTR_NONCE_SIZE: u64 = 16;

/// Returns the encrypted file size for a given plaintext size (CTR adds only the nonce)
pub fn ctr_encrypted_size(plaintext_size: u64) -> u64 {
    plaintext_size + CTR_NONCE_SIZE
}

/// AsyncWrite wrapper that encrypts data using AES-256-CTR.
/// Writes a 16-byte random nonce first, then encrypts all subsequent data.
pub struct CtrEncryptWriter<W: AsyncWrite + Unpin> {
    writer: W,
    cipher: Aes256Ctr,
    nonce: [u8; 16],
    header_written: bool,
    /// Tracks total plaintext bytes encrypted so far, used to seek cipher back on partial writes.
    bytes_encrypted: u64,
}

impl<W: AsyncWrite + Unpin> CtrEncryptWriter<W> {
    pub fn new(writer: W, key: &[u8; 32]) -> RsResult<Self> {
        let nonce = random_iv();
        let cipher = Aes256Ctr::new(key.into(), &nonce.into());
        Ok(Self {
            writer,
            cipher,
            nonce,
            header_written: false,
            bytes_encrypted: 0,
        })
    }

    /// Write the nonce header if not yet written. Returns Poll::Pending if writer not ready.
    fn poll_write_header(&mut self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.header_written {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.writer).poll_write(cx, &self.nonce) {
            Poll::Ready(Ok(n)) if n == 16 => {
                self.header_written = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Ok(_)) => {
                // Partial nonce write - this is unlikely with buffered writers but handle it
                Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "Failed to write full nonce",
                )))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for CtrEncryptWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // Ensure header is written first
        match self.poll_write_header(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        }

        // Encrypt data (CTR is XOR-based, same operation for encrypt/decrypt)
        let pos_before = self.bytes_encrypted;
        let mut encrypted = buf.to_vec();
        self.cipher.apply_keystream(&mut encrypted);

        match Pin::new(&mut self.writer).poll_write(cx, &encrypted) {
            Poll::Ready(Ok(n)) => {
                self.bytes_encrypted = pos_before + n as u64;
                if n < encrypted.len() {
                    // Partial write: cipher advanced past what was written.
                    // Seek it back to match the actual bytes written.
                    let pos = self.bytes_encrypted;
                    self.cipher.seek(pos);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => {
                // Nothing written: rewind cipher to before this call
                self.cipher.seek(pos_before);
                Poll::Ready(Err(e))
            }
            Poll::Pending => {
                // Nothing written: rewind cipher to before this call
                self.cipher.seek(pos_before);
                Poll::Pending
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

/// AsyncRead wrapper that decrypts data using AES-256-CTR.
/// Reads the 16-byte nonce from the stream on first read, then decrypts transparently.
pub struct CtrDecryptReader<R: AsyncRead + Unpin> {
    reader: R,
    cipher: Option<Aes256Ctr>,
    key: [u8; 32],
    nonce_buf: Vec<u8>,
    nonce_read: bool,
}

impl<R: AsyncRead + Unpin> CtrDecryptReader<R> {
    /// Create a new decrypting reader. The nonce is read from the first 16 bytes of the stream.
    pub fn new(reader: R, key: &[u8; 32]) -> Self {
        Self {
            reader,
            cipher: None,
            key: *key,
            nonce_buf: Vec::with_capacity(16),
            nonce_read: false,
        }
    }

    /// Create a decrypting reader for range requests where the nonce is already known.
    /// `offset` is the plaintext byte offset we want to start decrypting from.
    /// The underlying reader should already be positioned past the nonce (at file offset = nonce_size + offset).
    pub fn new_at_offset(reader: R, key: &[u8; 32], nonce: &[u8; 16], offset: u64) -> Self {
        let mut cipher = Aes256Ctr::new(key.into(), nonce.into());
        // Seek the CTR counter to the correct position
        cipher.seek(offset);
        Self {
            reader,
            cipher: Some(cipher),
            key: *key,
            nonce_buf: Vec::new(),
            nonce_read: true,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CtrDecryptReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Phase 1: Read the nonce from the stream
        if !self.nonce_read {
            let remaining = 16 - self.nonce_buf.len();
            let mut nonce_tmp = vec![0u8; remaining];
            let mut nonce_read_buf = ReadBuf::new(&mut nonce_tmp);
            match Pin::new(&mut self.reader).poll_read(cx, &mut nonce_read_buf) {
                Poll::Ready(Ok(())) => {
                    let filled = nonce_read_buf.filled().len();
                    if filled == 0 {
                        // EOF before nonce complete
                        return Poll::Ready(Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "Encrypted file too short: missing nonce",
                        )));
                    }
                    self.nonce_buf.extend_from_slice(&nonce_tmp[..filled]);
                    if self.nonce_buf.len() == 16 {
                        let nonce: [u8; 16] = self.nonce_buf[..16].try_into().unwrap();
                        self.cipher = Some(Aes256Ctr::new(self.key.as_ref().into(), &nonce.into()));
                        self.nonce_read = true;
                    }
                    // If nonce not complete yet, we need another read
                    if !self.nonce_read {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    // Nonce read complete, fall through to read actual data
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        // Phase 2: Read and decrypt data
        let before = buf.filled().len();
        match Pin::new(&mut self.reader).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let after = buf.filled().len();
                let new_bytes = after - before;
                if new_bytes > 0 {
                    // Decrypt the newly read bytes in place
                    let cipher = self.cipher.as_mut().unwrap();
                    cipher.apply_keystream(&mut buf.filled_mut()[before..after]);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
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

    #[tokio::test]
    async fn ctr_roundtrip() {
        use tokio::io::{copy, AsyncReadExt};

        let key = derive_key("test-ctr-password".to_string());
        let plaintext = b"Hello, this is a test of CTR encryption with streaming!";

        // Encrypt
        let mut encrypted = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut encrypted);
            let mut writer = CtrEncryptWriter::new(cursor, &key).unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut writer, plaintext).await.unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut writer).await.unwrap();
        }

        // Verify encrypted size = plaintext + 16 nonce
        assert_eq!(encrypted.len(), plaintext.len() + 16);
        // Verify data is actually encrypted (not plaintext)
        assert_ne!(&encrypted[16..], plaintext.as_slice());

        // Decrypt
        let reader = std::io::Cursor::new(&encrypted);
        let mut decryptor = CtrDecryptReader::new(reader, &key);
        let mut decrypted = Vec::new();
        decryptor.read_to_end(&mut decrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn ctr_range_decrypt() {
        use tokio::io::AsyncReadExt;

        let key = derive_key("test-ctr-range".to_string());
        let plaintext: Vec<u8> = (0..=255u8).cycle().take(1024).collect();

        // Encrypt
        let mut encrypted = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut encrypted);
            let mut writer = CtrEncryptWriter::new(cursor, &key).unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut writer, &plaintext).await.unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut writer).await.unwrap();
        }

        // Extract nonce
        let nonce: [u8; 16] = encrypted[..16].try_into().unwrap();

        // Decrypt from offset 100
        let offset: u64 = 100;
        let range_data = &encrypted[(16 + offset as usize)..];
        let reader = std::io::Cursor::new(range_data);
        let mut decryptor = CtrDecryptReader::new_at_offset(reader, &key, &nonce, offset);
        let mut decrypted = Vec::new();
        decryptor.read_to_end(&mut decrypted).await.unwrap();

        assert_eq!(decrypted, &plaintext[offset as usize..]);
    }

}