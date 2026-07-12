use std::env;
use std::io::{BufRead, BufReader};
use std::path::Path;

fn main() {
    tauri_build::build();
    embed_credentials();
}

fn embed_credentials() {
    // Look for .env in workspace root (one level above src-tauri/).
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let env_file = Path::new(&manifest_dir).parent().unwrap_or(Path::new(".")).join(".env");
    if env_file.exists() {
        load_env_file(&env_file);
        println!("cargo:rerun-if-changed={}", env_file.display());
    }
    println!("cargo:rerun-if-env-changed=GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=GOOGLE_CLIENT_SECRET");

    let client_id = env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let client_secret = env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();

    // Random key generated fresh each build — different binary layout every time.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let key = gen_key(nanos);

    let enc_id = xor_bytes(client_id.as_bytes(), &key);
    let enc_secret = xor_bytes(client_secret.as_bytes(), &key);

    let out_dir = env::var("OUT_DIR").unwrap();
    let content = format!(
        "pub(super) const _CRED_KEY: &[u8] = &{key:?};\n\
         pub(super) const _ENC_CLIENT_ID: &[u8] = &{enc_id:?};\n\
         pub(super) const _ENC_CLIENT_SECRET: &[u8] = &{enc_secret:?};\n"
    );
    std::fs::write(format!("{out_dir}/credentials.rs"), content)
        .expect("failed to write credentials.rs");
}

fn load_env_file(path: &Path) {
    let Ok(file) = std::fs::File::open(path) else { return };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let val = val.trim().trim_matches('"').trim_matches('\'');
            // Only set if not already provided by the real environment.
            if env::var(key.trim()).is_err() {
                // SAFETY: single-threaded build script.
                unsafe { env::set_var(key.trim(), val) };
            }
        }
    }
}

// Simple LCG — produces 32 pseudo-random bytes from a nanosecond seed.
fn gen_key(seed: u32) -> Vec<u8> {
    let mut state = seed as u64 ^ 0x517cc1b727220a95;
    (0..32)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) ^ state) as u8
        })
        .collect()
}

fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .zip(key.iter().cycle())
        .map(|(b, k)| b ^ k)
        .collect()
}
