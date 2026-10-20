//! Offline check of the public Windows release artifacts. Never runs the installer.
//!
//! Mirrors tauri-plugin-updater's Base64 envelopes and Minisign verification:
//! https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/updater/src/updater.rs

use std::{env, fs, path::PathBuf, process::ExitCode};

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const EMBEDDED_CONFIG: &str = include_str!("../tauri.conf.json");

fn decode_envelope(encoded: &str, label: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| format!("Envelope Base64 invalido: {label}."))?;
    String::from_utf8(bytes).map_err(|_| format!("Envelope UTF-8 invalido: {label}."))
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Campo obrigatorio ausente: {pointer}."))
}

fn modified_signature(signature_text: &str) -> Result<Signature, String> {
    let mut lines: Vec<String> = signature_text.lines().map(str::to_owned).collect();
    let signature_line = lines.get_mut(1).ok_or("Assinatura Minisign incompleta.")?;
    let mut raw = STANDARD
        .decode(signature_line.as_bytes())
        .map_err(|_| "Assinatura Minisign interna invalida.")?;
    // Preserve the algorithm and key id; change one byte in the signature itself.
    let byte = raw.get_mut(20).ok_or("Assinatura Minisign curta.")?;
    *byte ^= 1;
    *signature_line = STANDARD.encode(raw);
    Signature::decode(&lines.join("\n"))
        .map_err(|_| "Falha ao construir a assinatura incorreta do teste.".to_owned())
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if !(2..=3).contains(&arguments.len()) {
        return Err("Uso: verify-installer <instalador.exe> <assinatura.sig> [latest.json]".into());
    }
    let installer_path = PathBuf::from(&arguments[0]);
    let signature_path = PathBuf::from(&arguments[1]);
    let mut installer = fs::read(&installer_path)
        .map_err(|error| format!("Nao foi possivel ler o instalador: {error}"))?;
    if !installer.starts_with(b"MZ") {
        return Err("O arquivo nao e um executavel Windows (cabecalho MZ ausente).".into());
    }
    let encoded_signature = fs::read_to_string(&signature_path)
        .map_err(|error| format!("Nao foi possivel ler a assinatura publica: {error}"))?;
    let config: Value =
        serde_json::from_str(EMBEDDED_CONFIG).map_err(|_| "Configuracao incorporada invalida.")?;
    let version = required_string(&config, "/version")?;
    let encoded_public_key = required_string(&config, "/plugins/updater/pubkey")?;
    let public_key_text = decode_envelope(encoded_public_key, "chave publica")?;
    let signature_text = decode_envelope(&encoded_signature, "assinatura publica")?;
    let public_key =
        PublicKey::decode(&public_key_text).map_err(|_| "Chave publica incorporada invalida.")?;
    let signature =
        Signature::decode(&signature_text).map_err(|_| "Assinatura Minisign invalida.")?;
    // Same compatibility flag as the Tauri updater; verifies both artifact and trusted comment.
    public_key
        .verify(&installer, &signature, true)
        .map_err(|_| "ASSINATURA REJEITADA: instalador ou assinatura nao correspondem a chave publica incorporada.")?;

    // A trusted comment is usable only after the global signature has been verified above.
    let signed_version = signature
        .trusted_comment()
        .split('\t')
        .find_map(|part| part.strip_prefix("version:"));
    if let Some(signed) = signed_version {
        if signed.trim_start_matches('v') != version.trim_start_matches('v') {
            return Err(
                "A versao assinada nao corresponde a versao incorporada no verificador.".into(),
            );
        }
    }

    let mut manifest_matches = None;
    if let Some(manifest_argument) = arguments.get(2) {
        let manifest: Value = serde_json::from_slice(
            &fs::read(PathBuf::from(manifest_argument))
                .map_err(|error| format!("Nao foi possivel ler o manifesto publico: {error}"))?,
        )
        .map_err(|_| "Manifesto de atualizacao invalido.")?;
        if required_string(&manifest, "/version")? != version {
            return Err("A versao do manifesto nao corresponde a configuracao incorporada.".into());
        }
        if required_string(&manifest, "/platforms/windows-x86_64/signature")?.trim()
            != encoded_signature.trim()
        {
            return Err("O manifesto nao contem a assinatura do instalador verificado.".into());
        }
        let expected_url = format!(
            "https://github.com/Vittuu/ivestfii/releases/download/v{version}/IvestFII-Setup.exe"
        );
        if required_string(&manifest, "/platforms/windows-x86_64/url")? != expected_url {
            return Err("A URL de download do manifesto nao corresponde a esta release.".into());
        }
        manifest_matches = Some(true);
    }

    let checksum = format!("{:x}", Sha256::digest(&installer));
    let size = installer.len();
    let corrupted_signature = modified_signature(&signature_text)?;
    if public_key
        .verify(&installer, &corrupted_signature, true)
        .is_ok()
    {
        return Err("FALHA: uma assinatura alterada foi aceita.".into());
    }
    // Mutate only the in-memory buffer. The installer on disk remains unchanged.
    installer[size / 2] ^= 1;
    if public_key.verify(&installer, &signature, true).is_ok() {
        return Err("FALHA: um instalador com um byte alterado foi aceito.".into());
    }

    println!(
        "{}",
        json!({
            "version": version,
            "installer": installer_path.file_name().map(|name| name.to_string_lossy()),
            "bytes": size,
            "sha256": checksum,
            "signatureVerified": true,
            "tamperedInstallerRejected": true,
            "tamperedSignatureRejected": true,
            "manifestMatches": manifest_matches,
            "signedVersion": signed_version,
        })
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
