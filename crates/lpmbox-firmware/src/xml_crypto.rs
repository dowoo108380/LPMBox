use aes::Aes256;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::NoPadding};
use lpmbox_core::{LpmError, Result};
use sha2::{Digest, Sha256};
use std::convert::TryInto;
use std::path::Path;

type Aes256CbcDec = cbc::Decryptor<Aes256>;

fn pbkdf1(password: &str, salt: &[u8], out_len: usize, iterations: usize) -> Vec<u8> {
    let mut seed = Vec::new();
    seed.extend_from_slice(password.as_bytes());
    seed.extend_from_slice(salt);

    let mut digest = Sha256::digest(&seed).to_vec();

    for _ in 1..iterations {
        digest = Sha256::digest(&digest).to_vec();
    }

    digest[..out_len].to_vec()
}

pub fn decrypt_scatter_x(path: &Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path)?;

    if data.len() < 64 {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid scatter.x: 파일 크기가 너무 작습니다.".to_string(),
        ));
    }

    let iv = &data[0..16];
    let salt = &data[16..32];
    let body = &data[32..];

    if body.len() % 16 != 0 {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid scatter.x: AES-CBC 블록 크기가 맞지 않습니다.".to_string(),
        ));
    }

    let key = pbkdf1("OSD", salt, 32, 1000);

    let mut plain_buf = body.to_vec();

    let plain = Aes256CbcDec::new_from_slices(&key, iv)
        .map_err(|err| LpmError::ScatterDecryptFailed(err.to_string()))?
        .decrypt_padded::<NoPadding>(&mut plain_buf)
        .map_err(|err| LpmError::ScatterDecryptFailed(err.to_string()))?
        .to_vec();

    if plain.len() < 48 {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid decrypted data: 복호화 결과가 너무 작습니다.".to_string(),
        ));
    }

    let size_bytes: [u8; 8] = plain[0..8]
        .try_into()
        .map_err(|_| LpmError::ScatterDecryptFailed("size 파싱 실패".to_string()))?;

    let size = i64::from_le_bytes(size_bytes);

    if size < 0 {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid decrypted data: payload size가 음수입니다.".to_string(),
        ));
    }

    let size = size as usize;

    let signature = &plain[8..16];
    let expected = [0xcf, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xfc];

    if signature != expected {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid signature".to_string(),
        ));
    }

    let payload_start = 16;
    let payload_end = payload_start + size;
    let digest_end = payload_end + 32;

    if digest_end > plain.len() {
        return Err(LpmError::ScatterDecryptFailed(
            "invalid decrypted data: payload 범위가 잘못되었습니다.".to_string(),
        ));
    }

    let payload = &plain[payload_start..payload_end];
    let digest = &plain[payload_end..digest_end];

    let actual_hash = Sha256::digest(payload);
    let actual_hash_slice: &[u8] = actual_hash.as_ref();

    if actual_hash_slice != digest {
        return Err(LpmError::ScatterDecryptFailed("hash mismatch".to_string()));
    }

    Ok(payload.to_vec())
}
