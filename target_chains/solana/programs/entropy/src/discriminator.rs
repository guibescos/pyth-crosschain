fn account_discriminator(ordinal: u64) -> [u8; 8] {
    ordinal.to_le_bytes()
}

pub fn config_discriminator() -> [u8; 8] {
    account_discriminator(0)
}

#[allow(dead_code)]
pub fn provider_discriminator() -> [u8; 8] {
    account_discriminator(1)
}

#[allow(dead_code)]
pub fn request_discriminator() -> [u8; 8] {
    account_discriminator(2)
}
