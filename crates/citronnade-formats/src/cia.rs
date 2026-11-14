//! CIA files: an installable title (what FBI and other installers take).
//!
//! A CIA holds, each on a 0x40 boundary: a 0x2020-byte header, the
//! certificate chain, the ticket (the licence), the TMD (the title's
//! metadata, with each content's SHA-256), the contents (here one: the
//! NCCH) and the metadata (the NCCH's dependency list, core version and
//! icon). The ticket's and TMD's signatures are placeholders, as makerom
//! writes them without keys, so consoles install these titles with
//! signature checks patched (as custom firmware does).
//!
//! The certificate chain is the retail one every CIA carries (CA00000003,
//! XS0000000c, CP0000000b), as makerom (Project_CTR, MIT licence) has it:
//! public certificates, which installers read to find the ticket's and the
//! TMD's issuers.

use alloc::vec;
use alloc::vec::Vec;

use crate::ncch;
use crate::sha256::sha256;

/// The retail certificate chain (0xA00 bytes).
pub const CERTIFICATE_CHAIN: &[u8] = include_bytes!("cia_certificates.bin");

const SIGNATURE_RSA2048_SHA256: u32 = 0x0001_0004;
/// The signature block before a ticket's or TMD's body.
const SIGNATURE_SIZE: usize = 4 + 0x100 + 0x3C;

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

/// A placeholder RSA-2048 SHA-256 signature block.
fn signature() -> Vec<u8> {
    let mut s = vec![0u8; SIGNATURE_SIZE];
    s[..4].copy_from_slice(&SIGNATURE_RSA2048_SHA256.to_be_bytes());
    s[4..0x104].fill(0xFF);
    s
}

fn issuer(name: &str) -> [u8; 0x40] {
    let mut out = [0u8; 0x40];
    out[..name.len()].copy_from_slice(name.as_bytes());
    out
}

/// The ticket: a permanent licence to content 0, without a title key (the
/// content is not encrypted).
fn ticket(title_id: u64, version: u16) -> Vec<u8> {
    let mut t = signature();
    let mut body = vec![0u8; 0x164];
    body[..0x40].copy_from_slice(&issuer("Root-CA00000003-XS0000000c"));
    body[0x7C] = 1; // format version
    // a ticket ID of this title's own
    body[0x90..0x98].copy_from_slice(&(0x0004_0000_0000_0000 | (title_id & 0xFFFF_FFFF_FFFF)).to_be_bytes());
    body[0x9C..0xA4].copy_from_slice(&title_id.to_be_bytes());
    body[0xA6..0xA8].copy_from_slice(&version.to_be_bytes());
    t.extend_from_slice(&body);
    // the content index: which contents the ticket covers (content 0)
    let words: [u32; 10] = [0x0001_0014, 0x28 + 0x84, 0x14, 0x0001_0014, 0, 0x28, 1, 0x84, 0x84, 0x0003_0000];
    for w in words {
        t.extend_from_slice(&w.to_be_bytes());
    }
    let mut segment = vec![0u8; 0x84];
    segment[4] = 1; // content 0 (level 0)
    t.extend_from_slice(&segment);
    t
}

/// The TMD: one content, its size and SHA-256.
fn tmd(title_id: u64, version: u16, content: &[u8], content_size: u64) -> Vec<u8> {
    let mut chunk = vec![0u8; 0x30];
    // content ID 0, index 0, no flags (not encrypted)
    chunk[8..16].copy_from_slice(&content_size.to_be_bytes());
    chunk[16..48].copy_from_slice(&sha256(content));
    let mut records = vec![0u8; 0x24 * 64];
    records[2..4].copy_from_slice(&1u16.to_be_bytes()); // first record: index 0, 1 content
    records[4..36].copy_from_slice(&sha256(&chunk));
    let mut body = vec![0u8; 0xC4];
    body[..0x40].copy_from_slice(&issuer("Root-CA00000003-CP0000000b"));
    body[0x40] = 1; // format version
    body[0x4C..0x54].copy_from_slice(&title_id.to_be_bytes());
    body[0x54..0x58].copy_from_slice(&0x40u32.to_be_bytes()); // a CTR title
    body[0x9C..0x9E].copy_from_slice(&version.to_be_bytes());
    body[0x9E..0xA0].copy_from_slice(&1u16.to_be_bytes()); // one content
    body[0xA4..0xC4].copy_from_slice(&sha256(&records));
    let mut t = signature();
    t.extend_from_slice(&body);
    t.extend_from_slice(&records);
    t.extend_from_slice(&chunk);
    t
}

/// The metadata: the dependency list, the core version and the icon.
fn metadata(ncch_image: &[u8], icon: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; 0x400];
    if let Some(deps) = ncch::dependency_list(ncch_image) {
        m[..0x180].copy_from_slice(deps);
    }
    m[0x300..0x304].copy_from_slice(&ncch::CORE_VERSION.to_le_bytes());
    m.extend_from_slice(icon);
    m
}

/// A CIA installing `ncch_image` (an application NCCH) as title `title_id`
/// at `version`, with `icon` (the SMDH) in its metadata.
pub fn build(ncch_image: &[u8], title_id: u64, version: u16, icon: &[u8]) -> Vec<u8> {
    let content_size = align(ncch_image.len(), 0x10);
    let mut content = ncch_image.to_vec();
    content.resize(content_size, 0);
    let ticket = ticket(title_id, version);
    let tmd = tmd(title_id, version, &content, content_size as u64);
    let meta = metadata(ncch_image, icon);

    let mut header = vec![0u8; 0x2020];
    header[0..4].copy_from_slice(&0x2020u32.to_le_bytes());
    header[8..12].copy_from_slice(&(CERTIFICATE_CHAIN.len() as u32).to_le_bytes());
    header[12..16].copy_from_slice(&(ticket.len() as u32).to_le_bytes());
    header[16..20].copy_from_slice(&(tmd.len() as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(meta.len() as u32).to_le_bytes());
    header[24..32].copy_from_slice(&(content_size as u64).to_le_bytes());
    header[0x20] = 0x80; // content 0

    let mut out = Vec::with_capacity(0x4000 + content_size + meta.len());
    for part in [&header[..], CERTIFICATE_CHAIN, &ticket, &tmd, &content, &meta] {
        out.resize(align(out.len(), 0x40), 0);
        out.extend_from_slice(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_le(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
    }

    #[test]
    fn layout() {
        let ncch_image = vec![7u8; 0x1234];
        let icon = vec![1u8; 0x36C0];
        let cia = build(&ncch_image, 0x0004_0000_0F81_2300, 0x0410, &icon);
        let (certs, tik, tmd_size, meta) = (u32_le(&cia, 8), u32_le(&cia, 12), u32_le(&cia, 16), u32_le(&cia, 20));
        assert_eq!((certs, tik, tmd_size, meta), (0xA00, 0x350, 0xB34, 0x400 + 0x36C0));
        let cert_at = align(0x2020, 0x40);
        let tik_at = align(cert_at + certs, 0x40);
        let tmd_at = align(tik_at + tik, 0x40);
        let content_at = align(tmd_at + tmd_size, 0x40);
        assert_eq!(&cia[cert_at..cert_at + 4], &0x0001_0003u32.to_be_bytes());
        // the ticket's and TMD's title IDs
        assert_eq!(&cia[tik_at + 0x140 + 0x9C..tik_at + 0x140 + 0xA4], &0x0004_0000_0F81_2300u64.to_be_bytes());
        assert_eq!(&cia[tmd_at + 0x140 + 0x4C..tmd_at + 0x140 + 0x54], &0x0004_0000_0F81_2300u64.to_be_bytes());
        // the content and its hash in the TMD's chunk
        let content = &cia[content_at..content_at + 0x1240];
        assert_eq!(&content[..0x1234], &ncch_image[..]);
        let chunk = tmd_at + 0x140 + 0xC4 + 0x24 * 64;
        assert_eq!(&cia[chunk + 16..chunk + 48], &sha256(content));
        assert_eq!(cia.len(), align(content_at + 0x1240, 0x40) + meta);
    }
}
