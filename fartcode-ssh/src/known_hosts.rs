//! Host key verification against `~/.ssh/known_hosts`.
//!
//! Closes the E12-01 ponytail (`check_server_key` accepted everything).
//! Policy is OpenSSH's `StrictHostKeyChecking=accept-new`:
//!
//! - known host, matching key → connect;
//! - unknown host → record the key, warn, connect (trust on first use);
//! - known host, DIFFERENT key → refuse — this is the MITM case, and no
//!   prompt can make "the host key changed" a safe yes;
//! - `@revoked` key → refuse.
//!
//! Matching covers the file's whole vocabulary: comma-separated glob
//! patterns (`*`/`?`), `!` negations, `[host]:port` brackets, and `|1|`
//! HMAC-SHA1 hashed hostnames. `@cert-authority` entries are skipped — CA
//! validation is not implemented, and skipping fails toward "unknown"
//! (recorded) rather than toward trusting an unverified CA.

use std::path::{Path, PathBuf};

use hmac::{Hmac, Mac};
use ssh_key::known_hosts::{HostPatterns, KnownHosts, Marker};
use ssh_key::PublicKey;
use tracing::{error, warn};

/// What the file says about one `(host, port, key)`.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// This exact key is on file for the host.
    Known,
    /// No entry mentions the host at all.
    Unknown,
    /// The host is on file with DIFFERENT key(s) — the algorithms seen.
    Mismatch(Vec<String>),
    /// This exact key is marked `@revoked`.
    Revoked,
}

/// The names OpenSSH would look this endpoint up under: the bare (lowered)
/// hostname on the default port, the bracketed `[host]:port` form otherwise
/// (both spellings on 22, since either may have been recorded).
fn candidates(host: &str, port: u16) -> Vec<String> {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        vec![host.clone(), format!("[{host}]:22")]
    } else {
        vec![format!("[{host}]:{port}")]
    }
}

/// OpenSSH-style glob over a hostname: `*` any run, `?` one char. Both
/// sides are already lowercased; plain iterative matcher with `*` backtrack.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi_, mut ni, mut star, mut back) = (0usize, 0usize, usize::MAX, 0usize);
    while ni < n.len() {
        if pi_ < p.len() && (p[pi_] == '?' || p[pi_] == n[ni]) {
            pi_ += 1;
            ni += 1;
        } else if pi_ < p.len() && p[pi_] == '*' {
            star = pi_;
            back = ni;
            pi_ += 1;
        } else if star != usize::MAX {
            pi_ = star + 1;
            back += 1;
            ni = back;
        } else {
            return false;
        }
    }
    while pi_ < p.len() && p[pi_] == '*' {
        pi_ += 1;
    }
    pi_ == p.len()
}

/// Whether an entry's host patterns cover any candidate name. A `!` match
/// vetoes the entry regardless of other patterns (OpenSSH semantics).
fn applies(patterns: &HostPatterns, cands: &[String]) -> bool {
    match patterns {
        HostPatterns::Patterns(pats) => {
            let mut hit = false;
            for raw in pats {
                let p = raw.to_ascii_lowercase();
                if let Some(neg) = p.strip_prefix('!') {
                    if cands.iter().any(|c| glob_match(neg, c)) {
                        return false;
                    }
                } else if cands.iter().any(|c| glob_match(&p, c)) {
                    hit = true;
                }
            }
            hit
        }
        HostPatterns::HashedName { salt, hash } => cands.iter().any(|c| {
            let mut mac =
                Hmac::<sha1::Sha1>::new_from_slice(salt).expect("hmac accepts any key length");
            mac.update(c.as_bytes());
            mac.finalize().into_bytes().as_slice() == hash
        }),
    }
}

/// Pure verdict over the file's text — the testable core.
///
/// Unparseable lines are skipped (OpenSSH tolerates junk); a `@revoked`
/// entry for a DIFFERENT key neither matches nor counts as a mismatch.
pub fn verify(known_hosts: &str, host: &str, port: u16, key: &PublicKey) -> Verdict {
    let cands = candidates(host, port);
    let mut other_algorithms = Vec::new();
    for entry in KnownHosts::new(known_hosts).flatten() {
        if !applies(entry.host_patterns(), &cands) {
            continue;
        }
        let same_key = entry.public_key().key_data() == key.key_data();
        match entry.marker() {
            Some(Marker::Revoked) if same_key => return Verdict::Revoked,
            Some(_) => continue, // revoked other key, or @cert-authority
            None if same_key => return Verdict::Known,
            None => other_algorithms.push(entry.public_key().algorithm().to_string()),
        }
    }
    if other_algorithms.is_empty() {
        Verdict::Unknown
    } else {
        Verdict::Mismatch(other_algorithms)
    }
}

/// `~/.ssh/known_hosts` — the same file OpenSSH reads and appends.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".ssh").join("known_hosts"))
}

/// Appends the key for this endpoint (accept-new's "new").
pub fn record(path: &Path, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let pattern = candidates(host, port).swap_remove(0);
    let openssh = key.to_openssh().map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{pattern} {openssh}")
}

/// The handler's entry point: read, verdict, act. `true` = proceed.
pub fn check(host: &str, port: u16, key: &PublicKey) -> bool {
    let Some(path) = default_path() else {
        // No `$HOME`, nowhere to pin — the pre-#99 behavior, said out loud.
        warn!(host, "no HOME; ssh host key NOT verified");
        return true;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let fingerprint = key.fingerprint(Default::default()).to_string();
    match verify(&text, host, port, key) {
        Verdict::Known => true,
        Verdict::Unknown => {
            if let Err(e) = record(&path, host, port, key) {
                warn!(host, error = %e, "could not record host key (accept-new)");
            }
            warn!(host, port, %fingerprint, "first connection — host key recorded (accept-new)");
            true
        }
        Verdict::Mismatch(expected) => {
            error!(
                host,
                port,
                offered = %fingerprint,
                expected = ?expected,
                file = %path.display(),
                "HOST KEY CHANGED — refusing connection (possible MITM); \
                 remove the stale known_hosts entry if the host was reinstalled"
            );
            false
        }
        Verdict::Revoked => {
            error!(host, port, %fingerprint, "host key is REVOKED — refusing connection");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::private::PrivateKey;
    use ssh_key::Algorithm;

    fn key(seed: u64) -> PublicKey {
        let mut rng = rand_like(seed);
        PrivateKey::random(&mut rng, Algorithm::Ed25519)
            .expect("keygen")
            .public_key()
            .clone()
    }

    /// Deterministic RngCore so tests need no rand dependency.
    fn rand_like(seed: u64) -> impl ssh_key::rand_core::CryptoRngCore {
        struct Lcg(u64);
        impl ssh_key::rand_core::RngCore for Lcg {
            fn next_u32(&mut self) -> u32 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (self.0 >> 32) as u32
            }
            fn next_u64(&mut self) -> u64 {
                ((self.next_u32() as u64) << 32) | self.next_u32() as u64
            }
            fn fill_bytes(&mut self, dest: &mut [u8]) {
                for chunk in dest.chunks_mut(4) {
                    let bytes = self.next_u32().to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
            }
            fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), ssh_key::rand_core::Error> {
                self.fill_bytes(dest);
                Ok(())
            }
        }
        impl ssh_key::rand_core::CryptoRng for Lcg {}
        Lcg(seed)
    }

    fn line(pattern: &str, key: &PublicKey) -> String {
        format!("{pattern} {}\n", key.to_openssh().unwrap())
    }

    #[test]
    fn known_unknown_and_mismatch() {
        let (ours, theirs) = (key(1), key(2));
        let file = line("build.example", &ours);

        assert_eq!(verify(&file, "build.example", 22, &ours), Verdict::Known);
        assert_eq!(verify(&file, "other.example", 22, &ours), Verdict::Unknown);
        // Same host, different key — the case the whole module exists for.
        assert!(matches!(
            verify(&file, "build.example", 22, &theirs),
            Verdict::Mismatch(_)
        ));
    }

    #[test]
    fn ports_use_the_bracket_form() {
        let k = key(3);
        let file = line("[build.example]:2222", &k);
        assert_eq!(verify(&file, "build.example", 2222, &k), Verdict::Known);
        // Same host on the default port is a DIFFERENT endpoint.
        assert_eq!(verify(&file, "build.example", 22, &k), Verdict::Unknown);
        // And a bracketed :22 entry still matches a port-22 dial.
        let file22 = line("[build.example]:22", &k);
        assert_eq!(verify(&file22, "build.example", 22, &k), Verdict::Known);
    }

    #[test]
    fn globs_negations_and_case() {
        let k = key(4);
        assert_eq!(
            verify(&line("*.example", &k), "BUILD.example", 22, &k),
            Verdict::Known
        );
        // A negation vetoes the entry even though the glob matches.
        assert_eq!(
            verify(
                &line("*.example,!build.example", &k),
                "build.example",
                22,
                &k
            ),
            Verdict::Unknown
        );
        assert_eq!(
            verify(&line("bu?ld.example", &k), "build.example", 22, &k),
            Verdict::Known
        );
    }

    #[test]
    fn hashed_entries_match_via_hmac() {
        let k = key(5);
        let salt = b"0123456789abcdef0123";
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(salt).unwrap();
        mac.update(b"build.example");
        let hash: [u8; 20] = mac.finalize().into_bytes().into();
        use base64ct::{Base64, Encoding};
        let pattern = format!(
            "|1|{}|{}",
            Base64::encode_string(salt),
            Base64::encode_string(&hash)
        );
        let file = line(&pattern, &k);
        assert_eq!(verify(&file, "build.example", 22, &k), Verdict::Known);
        assert_eq!(verify(&file, "other.example", 22, &k), Verdict::Unknown);
    }

    #[test]
    fn revoked_refuses_and_junk_lines_are_skipped() {
        let k = key(6);
        let file = format!(
            "# comment\nnot a valid line\n@revoked build.example {}",
            k.to_openssh().unwrap()
        );
        assert_eq!(verify(&file, "build.example", 22, &k), Verdict::Revoked);
        // A revoked entry for a DIFFERENT key is neither a match nor a mismatch.
        assert_eq!(
            verify(&file, "build.example", 22, &key(7)),
            Verdict::Unknown
        );
    }

    #[test]
    fn record_appends_a_line_verify_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let k = key(8);

        record(&path, "Build.Example", 2222, &k).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[build.example]:2222 "), "{text}");
        assert_eq!(verify(&text, "build.example", 2222, &k), Verdict::Known);

        // Append, not truncate: a second host keeps the first.
        record(&path, "other.example", 22, &key(9)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(verify(&text, "build.example", 2222, &k), Verdict::Known);
    }
}
