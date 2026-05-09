//! `zad signing` — manage the local Ed25519 signing key.
//!
//! The signing key lives in the OS keychain under
//! `secrets:signing:v1`. It is the single root of trust used to
//! self-sign the per-machine trust store at
//! `~/.zad/signing/trusted.toml` and to authorize permission file
//! loads.
//!
//! `zad signing init` is the **only** code path that mints a fresh
//! key. Every other sign/verify path uses
//! [`zad::permissions::signing::require_keychain_key`] and fails
//! closed when the keychain has no entry. This prevents an agent
//! from silently bootstrapping its own root of trust by simply
//! running `zad <svc> permissions sign` on a fresh machine.

use clap::{Args, Subcommand};
use serde::Serialize;

use zad::error::Result;
use zad::permissions::signing;
use zad::permissions::trust::{TrustStore, trust_store_path};

#[derive(Debug, Args)]
pub struct SigningArgs {
    #[command(subcommand)]
    pub action: SigningAction,
}

#[derive(Debug, Subcommand)]
pub enum SigningAction {
    /// Bootstrap the local signing key. Mints a fresh Ed25519 keypair
    /// in the OS keychain and initializes an empty signed trust
    /// store. Safe to run on a machine that already has a key (idempotent
    /// without --force).
    Init(InitArgs),
    /// Print the local signing key's fingerprint and the paths of the
    /// public-key cache and trust store.
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Rotate the keychain key, replacing any existing entry. The
    /// trust store is reset (every existing trust entry was signed by
    /// the old key and would fail verification under the new key);
    /// you must re-sign every permissions file you want loaded.
    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: SigningArgs) -> Result<()> {
    match args.action {
        SigningAction::Init(a) => run_init(a),
        SigningAction::Show(a) => run_show(a),
    }
}

#[derive(Debug, Serialize)]
struct InitOut {
    command: &'static str,
    fingerprint: String,
    public_key: String,
    rotated: bool,
    trust_store_path: String,
    public_key_cache_path: String,
}

fn run_init(args: InitArgs) -> Result<()> {
    let existed = signing::load_from_keychain()?.is_some();
    let key = if args.force {
        signing::rotate_keychain_key()?
    } else {
        signing::load_or_create_from_keychain()?
    };

    signing::write_public_key_cache(&key)?;

    // On --force we deliberately discard any existing trust store —
    // every entry it held was signed by the now-rotated key and would
    // fail verification anyway.
    if args.force {
        let path = trust_store_path()?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| zad::error::ZadError::Io {
                path: path.clone(),
                source: e,
            })?;
        }
    }

    // Initialize the trust store (empty + self-signed) so subsequent
    // `permissions sign` / `permissions commit` calls succeed without
    // bumping into a missing-store branch.
    let store = TrustStore::default();
    store.save(&key)?;

    let out = InitOut {
        command: "signing.init",
        fingerprint: key.fingerprint(),
        public_key: key.public_key_b64(),
        rotated: args.force && existed,
        trust_store_path: trust_store_path()?.display().to_string(),
        public_key_cache_path: signing::public_key_cache_path()?.display().to_string(),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        if out.rotated {
            println!("Rotated signing key (fingerprint: {}).", out.fingerprint);
            println!(
                "  trust store reset: every permissions file must be re-signed via `zad <service> permissions sign`."
            );
        } else if existed {
            println!(
                "Signing key already initialized (fingerprint: {}).",
                out.fingerprint
            );
        } else {
            println!(
                "Initialized signing key (fingerprint: {}).",
                out.fingerprint
            );
        }
        println!("  trust store     : {}", out.trust_store_path);
        println!("  public-key cache: {}", out.public_key_cache_path);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ShowOut {
    command: &'static str,
    initialized: bool,
    fingerprint: Option<String>,
    public_key: Option<String>,
    trust_store_path: String,
    public_key_cache_path: String,
}

fn run_show(args: ShowArgs) -> Result<()> {
    let key = signing::load_from_keychain()?;
    let out = ShowOut {
        command: "signing.show",
        initialized: key.is_some(),
        fingerprint: key.as_ref().map(|k| k.fingerprint()),
        public_key: key.as_ref().map(|k| k.public_key_b64()),
        trust_store_path: trust_store_path()?.display().to_string(),
        public_key_cache_path: signing::public_key_cache_path()?.display().to_string(),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if out.initialized {
        println!("Signing key: {}", out.fingerprint.as_deref().unwrap_or(""));
        println!("  trust store     : {}", out.trust_store_path);
        println!("  public-key cache: {}", out.public_key_cache_path);
    } else {
        println!("No signing key initialized.");
        println!("  bootstrap with: zad signing init");
    }
    Ok(())
}
