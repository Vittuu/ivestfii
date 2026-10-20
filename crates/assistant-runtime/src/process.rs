use crate::{AdapterConfig, AdapterIo, LaunchSpec, ProviderEvent};
use ivestfii_assistant::detect_providers;
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 2 * 1024 * 1024;
const TURN_TIMEOUT: Duration = Duration::from_secs(600);

#[path = "mcp_client.rs"]
mod mcp_client;

pub(crate) fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn new(child: &Child) -> Result<Self, String> {
        use std::{
            mem::{size_of, zeroed},
            os::windows::io::AsRawHandle,
        };
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        // The handle is owned here, never shared; closing it kills just this CLI
        // and its MCP descendants, including on error/cancellation/host exit.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                let error = std::io::Error::last_os_error().to_string();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(Self(handle))
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub(crate) struct OwnedChild {
    child: Child,
    #[cfg(windows)]
    job: Option<Job>,
}
impl OwnedChild {
    fn spawn(command: &mut Command) -> Result<Self, String> {
        hidden(command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar a CLI: {e}"))?;
        #[cfg(windows)]
        let job = match Job::new(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Falha ao supervisionar a CLI: {error}"));
            }
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum Frame {
    Json(Value),
    Error(String),
    Eof,
}
type InputFrame = (Vec<u8>, mpsc::Sender<Result<(), String>>);

fn read_frames(output: impl Read, tx: mpsc::SyncSender<Frame>, label: &'static str) {
    let mut reader = BufReader::new(output);
    loop {
        let mut bytes = Vec::new();
        match reader
            .by_ref()
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) => {
                let _ = tx.send(Frame::Eof);
                break;
            }
            Ok(_) if bytes.len() > MAX_FRAME => {
                let _ = tx.send(Frame::Error(format!(
                    "{label} excedeu o limite de mensagem."
                )));
                break;
            }
            Ok(_) => {
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let frame = match serde_json::from_slice(&bytes) {
                    Ok(value) => Frame::Json(value),
                    Err(_) => {
                        let _ = tx.send(Frame::Error(format!(
                            "{label} retornou uma mensagem fora do protocolo JSON esperado."
                        )));
                        break;
                    }
                };
                if tx.send(frame).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = tx.send(Frame::Error(format!("Falha ao ler {label}: {error}")));
                break;
            }
        }
    }
}

/// Shared bounded, cancellable NDJSON transport. Each owned process is in its
/// own Windows Job, so closing a turn also closes every managed descendant.
struct JsonProcess {
    process: OwnedChild,
    input: mpsc::SyncSender<InputFrame>,
    frames: mpsc::Receiver<Frame>,
    cancel: Arc<AtomicBool>,
    turn_deadline: Instant,
    operation_deadline: Option<Instant>,
    label: &'static str,
}

impl JsonProcess {
    fn spawn(
        spec: &LaunchSpec,
        directory: &std::path::Path,
        cancel: Arc<AtomicBool>,
        turn_deadline: Instant,
        label: &'static str,
        inherit_cli_environment: bool,
    ) -> Result<Self, String> {
        if cfg!(windows)
            && spec
                .executable
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        {
            return Err(
                "É necessário o executável nativo da CLI; o atalho de shell não é compatível."
                    .into(),
            );
        }
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.args)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !inherit_cli_environment {
            // The local read-only MCP needs no provider authentication or
            // application environment. Keep only OS runtime/temp locations.
            command.env_clear();
            for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
        // Preserve the official CLI login, while preventing an ambient API key
        // from unexpectedly replacing the user's subscription with API billing.
        for key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "GOOGLE_API_KEY",
            "GEMINI_API_KEY",
        ] {
            command.env_remove(key);
        }
        for key in [
            "CODEX_APP_TOOLS_PIPE_PATH",
            "CODEX_MCP_NODE_PATH",
            "CODEX_THREAD_ID",
            "CODEX_SESSION_ID",
            "CODEX_PERMISSION_PROFILE",
        ] {
            command.env_remove(key);
        }
        for (key, value) in &spec.environment {
            command.env(key, value);
        }
        let mut process = OwnedChild::spawn(&mut command)?;
        let mut stdin = process
            .child
            .stdin
            .take()
            .ok_or("CLI sem entrada padrão.")?;
        let (input, input_rx) = mpsc::sync_channel::<InputFrame>(1);
        thread::spawn(move || {
            while let Ok((bytes, reply)) = input_rx.recv() {
                let result = stdin
                    .write_all(&bytes)
                    .and_then(|_| stdin.flush())
                    .map_err(|e| format!("A CLI encerrou a entrada: {e}"));
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let output = process.child.stdout.take().ok_or("CLI sem saída padrão.")?;
        let diagnostic = process
            .child
            .stderr
            .take()
            .ok_or("CLI sem saída de diagnóstico.")?;
        let (tx, frames) = mpsc::sync_channel(64);
        thread::spawn(move || read_frames(output, tx, label));
        // Drain diagnostics without unbounded buffering or recording private CLI
        // environment/log data. Protocol error messages provide user feedback.
        thread::spawn(move || {
            let _ = std::io::copy(&mut BufReader::new(diagnostic), &mut std::io::sink());
        });
        Ok(Self {
            process,
            input,
            frames,
            cancel,
            turn_deadline,
            operation_deadline: None,
            label,
        })
    }

    fn check_active(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err("Resposta cancelada.".into());
        }
        let now = Instant::now();
        if now >= self.turn_deadline {
            return Err("O turno excedeu 10 minutos; o histórico foi preservado.".into());
        }
        if self
            .operation_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return Err(format!("{} excedeu o prazo da operação.", self.label));
        }
        Ok(())
    }

    fn send(&mut self, value: &Value) -> Result<(), String> {
        self.check_active()?;
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME {
            return Err("Entrada acima do limite de protocolo.".into());
        }
        bytes.push(b'\n');
        let (tx, rx) = mpsc::channel();
        let mut pending = (bytes, tx);
        loop {
            self.check_active()?;
            match self.input.try_send(pending) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(frame)) => {
                    pending = frame;
                    thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(format!("{} encerrou a entrada.", self.label))
                }
            }
        }
        loop {
            self.check_active()?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(format!("{} encerrou a entrada.", self.label)),
            }
        }
    }
    fn receive(&mut self) -> Result<Value, String> {
        loop {
            self.check_active()?;
            match self.frames.recv_timeout(Duration::from_millis(100)) {
                Ok(Frame::Json(value)) => return Ok(value),
                Ok(Frame::Error(error)) => return Err(error),
                Ok(Frame::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status = self.process.child.try_wait().ok().flatten();
                    return Err(format!("{} encerrou sem concluir a resposta{}; confira a versão compatível e o login no terminal.",self.label,status.map(|s|format!(" ({s})")).unwrap_or_default()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl mcp_client::Transport for JsonProcess {
    fn send(&mut self, value: &Value) -> Result<(), String> {
        JsonProcess::send(self, value)
    }
    fn receive(&mut self) -> Result<Value, String> {
        JsonProcess::receive(self)
    }
    fn set_timeout(&mut self, timeout: Duration) {
        self.operation_deadline = Some(Instant::now() + timeout);
    }
}

pub(crate) struct ProcessIo<'a> {
    cli: JsonProcess,
    mcp: Option<mcp_client::Client<JsonProcess>>,
    emit: &'a mut dyn FnMut(ProviderEvent) -> Result<(), String>,
}

impl<'a> ProcessIo<'a> {
    pub fn spawn(
        spec: &LaunchSpec,
        directory: &std::path::Path,
        cancel: Arc<AtomicBool>,
        emit: &'a mut dyn FnMut(ProviderEvent) -> Result<(), String>,
    ) -> Result<Self, String> {
        let cli = JsonProcess::spawn(
            spec,
            directory,
            cancel,
            Instant::now() + TURN_TIMEOUT,
            "A CLI",
            true,
        )?;
        Ok(Self {
            cli,
            mcp: None,
            emit,
        })
    }

    /// Start one managed local MCP for the entire turn. The provider never
    /// receives process execution privileges, private environment or keys.
    pub fn configure_mcp(&mut self, config: &AdapterConfig) -> Result<(), String> {
        self.cli.check_active()?;
        if self.mcp.is_some() {
            return Err("O MCP gerenciado já foi configurado neste turno.".into());
        }
        let spec = LaunchSpec {
            executable: config.mcp_executable.clone(),
            args: config.mcp_arguments.clone(),
            environment: Vec::new(),
        };
        let transport = JsonProcess::spawn(
            &spec,
            &config.work_dir,
            self.cli.cancel.clone(),
            self.cli.turn_deadline,
            "O MCP local",
            false,
        )?;
        // A failed handshake drops its owned subprocess immediately.
        self.mcp = Some(mcp_client::Client::connect(transport)?);
        Ok(())
    }
}

impl AdapterIo for ProcessIo<'_> {
    fn send(&mut self, value: &Value) -> Result<(), String> {
        self.cli.send(value)
    }
    fn receive(&mut self) -> Result<Value, String> {
        self.cli.receive()
    }
    fn emit(&mut self, event: ProviderEvent) -> Result<(), String> {
        (self.emit)(event)
    }
    fn call_tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        self.cli.check_active()?;
        let result = self
            .mcp
            .as_mut()
            .ok_or("O MCP local não foi configurado para este turno.")?
            .call_tool(name, args);
        if self.mcp.as_ref().is_some_and(|client| !client.is_usable()) {
            // Fatal protocol/transport errors close the managed process now,
            // rather than leaving it around until a model finishes explaining.
            self.mcp.take();
        }
        result
    }
    fn tool_definitions(&mut self) -> Result<Value, String> {
        self.cli.check_active()?;
        self.mcp
            .as_ref()
            .ok_or("O MCP local não foi configurado para este turno.")?
            .definitions()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbedProvider {
    #[serde(flatten)]
    pub provider: ivestfii_assistant::ProviderStatus,
    pub ready: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

pub fn probe_providers() -> Vec<ProbedProvider> {
    detect_providers()
        .into_iter()
        .map(|provider| {
            let result = provider
                .executable
                .as_ref()
                .ok_or_else(|| {
                    "CLI não encontrada. Instale e faça login pelo terminal.".to_string()
                })
                .and_then(|path| probe(path));
            let (ready, version, error) = match result {
                Ok(v) => (true, Some(v), None),
                Err(e) => (false, None, Some(e)),
            };
            ProbedProvider {
                provider,
                ready,
                version,
                error,
            }
        })
        .collect()
}

fn probe(path: &str) -> Result<String, String> {
    if cfg!(windows) && !path.to_ascii_lowercase().ends_with(".exe") {
        return Err("Atalho encontrado, mas falta o executável nativo da CLI.".into());
    }
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut process = OwnedChild::spawn(&mut command)?;
    let output = process
        .child
        .stdout
        .take()
        .ok_or("CLI sem saída de versão.")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let result = output.take(4096).read_to_string(&mut text).map(|_| text);
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = process.child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(
                    "A CLI encontrada falhou ao informar a versão; verifique a instalação.".into(),
                );
            }
            let version = rx
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| "A CLI não informou a versão.")?
                .map_err(|e| e.to_string())?;
            if version.trim().is_empty() {
                return Err("A CLI não informou a versão.".into());
            }
            return Ok(version.trim().chars().take(160).collect());
        }
        if Instant::now() > deadline {
            return Err("A CLI não respondeu à verificação de versão.".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn sleeping_cli() -> LaunchSpec {
        LaunchSpec {
            executable: std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            args: vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "Start-Sleep -Seconds 30".into(),
            ],
            environment: vec![],
        }
    }
    #[test]
    fn cancel_interrupts_waiting_for_cli_output_and_kills_owned_process() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut emit = |_| Ok(());
        let mut io =
            ProcessIo::spawn(&sleeping_cli(), temp.path(), cancel.clone(), &mut emit).unwrap();
        let signal = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert!(io.receive().unwrap_err().contains("cancelada"));
        drop(io);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn cancel_interrupts_a_full_stdin_pipe() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut emit = |_| Ok(());
        let mut io =
            ProcessIo::spawn(&sleeping_cli(), temp.path(), cancel.clone(), &mut emit).unwrap();
        let signal = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert!(io
            .send(&serde_json::json!({"prompt":"x".repeat(1_000_000)}))
            .unwrap_err()
            .contains("cancelada"));
        drop(io);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn operation_deadline_interrupts_a_full_pipe() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut emit = |_| Ok(());
        let mut io = ProcessIo::spawn(&sleeping_cli(), temp.path(), cancel, &mut emit).unwrap();
        io.cli.operation_deadline = Some(Instant::now() + Duration::from_millis(100));
        let started = Instant::now();
        assert!(io
            .send(&serde_json::json!({"prompt":"x".repeat(1_000_000)}))
            .unwrap_err()
            .contains("prazo"));
        drop(io);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn cancelled_managed_mcp_handshake_returns_and_drops_child() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut emit = |_| Ok(());
        let spec = sleeping_cli();
        let config = AdapterConfig {
            executable: spec.executable.clone(),
            work_dir: temp.path().to_path_buf(),
            mcp_executable: spec.executable.clone(),
            mcp_arguments: spec.args.clone(),
            data_dir: temp.path().to_path_buf(),
            conversation_id: "synthetic".into(),
            context: "synthetic".into(),
            model: None,
        };
        let mut io = ProcessIo::spawn(&spec, temp.path(), cancel.clone(), &mut emit).unwrap();
        assert!(io
            .tool_definitions()
            .unwrap_err()
            .contains("não foi configurado"));
        assert!(io
            .call_tool("carteira_snapshot", &serde_json::json!({}))
            .unwrap_err()
            .contains("não foi configurado"));
        let signal = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            signal.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert!(io.configure_mcp(&config).unwrap_err().contains("cancelada"));
        assert!(io.mcp.is_none());
        drop(io);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_ndjson_and_final_frame_without_newline() {
        let (tx, rx) = mpsc::sync_channel(8);
        read_frames(Cursor::new(b"\n {\"id\":1}\n{\"id\":2}"), tx, "MCP");
        assert!(matches!(rx.recv().unwrap(),Frame::Json(value) if value["id"]==1));
        assert!(matches!(rx.recv().unwrap(),Frame::Json(value) if value["id"]==2));
        assert!(matches!(rx.recv().unwrap(), Frame::Eof));
    }

    #[test]
    fn rejects_frames_over_two_mebibytes_before_parsing() {
        let (tx, rx) = mpsc::sync_channel(8);
        read_frames(Cursor::new(vec![b'x'; MAX_FRAME + 2]), tx, "MCP");
        assert!(matches!(rx.recv().unwrap(),Frame::Error(message) if message.contains("limite")));
    }

    #[test]
    fn malformed_json_or_utf8_is_terminal() {
        for bytes in [b"not json\n{\"id\":2}".to_vec(), vec![0xff, b'\n']] {
            let (tx, rx) = mpsc::sync_channel(8);
            read_frames(Cursor::new(bytes), tx, "MCP");
            assert!(matches!(rx.recv().unwrap(),Frame::Error(message) if message.contains("JSON")));
            assert!(rx.try_recv().is_err());
        }
    }
}
