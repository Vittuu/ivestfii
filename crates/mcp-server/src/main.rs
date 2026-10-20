use ivestfii_mcp::{run, McpServer};
use std::{env, io, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let (data_dir, wallet_id) = match parse_arguments(env::args_os().skip(1)) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "Uso: ivestfii-mcp --data-dir <diretório com portfolio.sqlite e assistant.sqlite> [--wallet-id <ID do catálogo>]"
            );
            return ExitCode::from(2);
        }
    };
    let mut server = match McpServer::open_for_wallet(&data_dir, wallet_id.as_deref()) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("Não foi possível iniciar o MCP local: {error}");
            return ExitCode::from(1);
        }
    };
    let input = io::stdin();
    let output = io::stdout();
    match run(&mut server, input.lock(), output.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Falha no transporte MCP: {error}");
            ExitCode::from(1)
        }
    }
}

fn parse_arguments(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(PathBuf, Option<String>), String> {
    let mut data_dir = None;
    let mut wallet_id = None;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--data-dir") if data_dir.is_none() => {
                let path = arguments.next().ok_or("Falta o diretório de dados.")?;
                if path.is_empty() || path.to_string_lossy().starts_with("--") {
                    return Err("Diretório de dados inválido.".into());
                }
                data_dir = Some(PathBuf::from(path));
            }
            Some("--wallet-id") if wallet_id.is_none() => {
                let id = arguments
                    .next()
                    .ok_or("Falta o ID da carteira.")?
                    .into_string()
                    .map_err(|_| "O ID da carteira deve ser texto UTF-8.".to_owned())?;
                if id.trim().is_empty() || id.starts_with("--") {
                    return Err("ID da carteira inválido.".into());
                }
                wallet_id = Some(id);
            }
            _ => return Err("Argumento desconhecido ou repetido.".into()),
        }
    }
    Ok((data_dir.ok_or("Informe --data-dir.".to_owned())?, wallet_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[&str]) -> Result<(PathBuf, Option<String>), String> {
        parse_arguments(input.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn accepts_pinned_wallet_in_either_argument_order() {
        assert_eq!(
            parse(&["--data-dir", "example", "--wallet-id", "principal"]).unwrap(),
            (PathBuf::from("example"), Some("principal".into()))
        );
        assert_eq!(
            parse(&["--wallet-id", "principal", "--data-dir", "example"]).unwrap(),
            (PathBuf::from("example"), Some("principal".into()))
        );
        assert_eq!(
            parse(&["--data-dir", "example"]).unwrap(),
            (PathBuf::from("example"), None)
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_arguments() {
        for arguments in [
            vec![],
            vec!["--data-dir"],
            vec!["--data-dir", "example", "--wallet-id"],
            vec!["--wallet-id", "principal"],
            vec!["--data-dir", "example", "--wallet-id", ""],
            vec!["--data-dir", "example", "--data-dir", "other"],
            vec![
                "--data-dir",
                "example",
                "--wallet-id",
                "principal",
                "--wallet-id",
                "other",
            ],
            vec!["--data-dir", "example", "--unknown", "value"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }
}
