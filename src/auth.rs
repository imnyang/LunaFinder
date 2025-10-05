use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use bcrypt::{hash, verify};
use sha2::{Digest, Sha256};

pub fn verify_password(password: &str, hashed: &str, algorithm: &str) -> bool {
    match algorithm.to_lowercase().as_str() {
        "argon2" => verify_argon2(password, hashed),
        "bcrypt" => verify_bcrypt(password, hashed),
        "sha256" => verify_sha256(password, hashed),
        "plain" => password == hashed,
        _ => false,
    }
}

pub fn hash_password(password: &str, algorithm: &str) -> Option<String> {
    match algorithm.to_lowercase().as_str() {
        "argon2" => hash_argon2(password),
        "bcrypt" => hash_bcrypt(password),
        "sha256" => Some(hash_sha256(password)),
        "plain" => Some(password.to_string()),
        _ => None,
    }
}

fn verify_argon2(password: &str, hashed: &str) -> bool {
    if let Ok(parsed_hash) = PasswordHash::new(hashed) {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    } else {
        false
    }
}

fn hash_argon2(password: &str) -> Option<String> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .ok()
        .map(|hash| hash.to_string())
}

fn verify_bcrypt(password: &str, hashed: &str) -> bool {
    verify(password, hashed).unwrap_or(false)
}

fn hash_bcrypt(password: &str) -> Option<String> {
    hash(password, 12).ok()
}

fn verify_sha256(password: &str, hashed: &str) -> bool {
    let computed = hash_sha256(password);
    computed == hashed
}

fn hash_sha256(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}
