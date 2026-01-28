use solana_program::hash::hashv;

fn account_discriminator(name: &[u8]) -> [u8; 8] {
    let hash = hashv(&[b"account:", name]);
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&hash.to_bytes()[..8]);
    discriminator
}

pub fn config_discriminator() -> [u8; 8] {
    account_discriminator(b"Config")
}

#[allow(dead_code)]
pub fn provider_discriminator() -> [u8; 8] {
    account_discriminator(b"Provider")
}

#[allow(dead_code)]
pub fn request_discriminator() -> [u8; 8] {
    account_discriminator(b"Request")
}
