use entropy::{
    constants::{COMMITMENT_METADATA_LEN, URI_LEN},
    instruction::RegisterProviderArgs,
};

#[allow(dead_code)]
pub fn build_register_args_with_metadata(
    fee_lamports: u64,
    commitment: [u8; 32],
    chain_length: u64,
    commitment_metadata: &[u8],
    uri: &[u8],
) -> RegisterProviderArgs {
    assert!(commitment_metadata.len() <= COMMITMENT_METADATA_LEN);
    assert!(uri.len() <= URI_LEN);

    let mut commitment_metadata_buf = [0u8; COMMITMENT_METADATA_LEN];
    commitment_metadata_buf[..commitment_metadata.len()].copy_from_slice(commitment_metadata);

    let mut uri_buf = [0u8; URI_LEN];
    uri_buf[..uri.len()].copy_from_slice(uri);

    RegisterProviderArgs {
        fee_lamports,
        commitment,
        commitment_metadata_len: commitment_metadata.len() as u16,
        _padding0: [0u8; 6],
        commitment_metadata: commitment_metadata_buf,
        chain_length,
        uri_len: uri.len() as u16,
        uri: uri_buf,
        _padding1: [0u8; 6],
    }
}

#[allow(dead_code)]
pub fn build_register_args(
    fee_lamports: u64,
    commitment: [u8; 32],
    chain_length: u64,
) -> RegisterProviderArgs {
    build_register_args_with_metadata(fee_lamports, commitment, chain_length, &[], &[])
}
