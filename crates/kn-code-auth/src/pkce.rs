use sha2::{Digest, Sha256};

pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
}

impl PkcePair {
    pub fn generate() -> anyhow::Result<Self> {
        let code_verifier = generate_random_string(64)?;
        let code_challenge = compute_challenge(&code_verifier);
        Ok(Self {
            code_verifier,
            code_challenge,
        })
    }
}

fn generate_random_string(length: usize) -> anyhow::Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    let mut bytes = vec![0u8; length];
    rng.fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("Failed to generate random bytes: {}", e))?;
    Ok(base64url_encode(&bytes))
}

fn compute_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let result = hasher.finalize();
    base64url_encode(&result)
}

fn base64url_encode(bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}
