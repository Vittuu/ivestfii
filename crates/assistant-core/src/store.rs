use crate::{
    context, ContextBundle, Conversation, Message, SourceReference, TaskCheckpoint, ToolEvent,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use std::{error::Error, fmt, path::Path, time::Duration};
use uuid::Uuid;

#[derive(Debug)]
pub enum AssistantError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Validation(String),
    NotFound(String),
}

impl fmt::Display for AssistantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(f, "Falha no histórico local: {error}"),
            Self::Io(error) => write!(f, "Falha ao abrir armazenamento local: {error}"),
            Self::Validation(error) => write!(f, "{error}"),
            Self::NotFound(error) => write!(f, "{error}"),
        }
    }
}

impl Error for AssistantError {}

impl From<rusqlite::Error> for AssistantError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

impl From<std::io::Error> for AssistantError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type AssistantResult<T> = Result<T, AssistantError>;

pub struct AssistantStore {
    connection: Connection,
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn conversation_from_row(row: &Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: row.get(0)?,
        title: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        provider: row.get(4)?,
        model: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn json_from_row(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let value: String = row.get(index)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskCheckpoint> {
    Ok(TaskCheckpoint {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        kind: row.get(2)?,
        status: row.get(3)?,
        checkpoint: json_from_row(row, 4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn tool_event_from_row(row: &Row<'_>) -> rusqlite::Result<ToolEvent> {
    let output: Option<String> = row.get(5)?;
    let output = output
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()?;
    Ok(ToolEvent {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        task_id: row.get(2)?,
        tool_name: row.get(3)?,
        input: json_from_row(row, 4)?,
        output,
        status: row.get(6)?,
        started_at: row.get(7)?,
        completed_at: row.get(8)?,
    })
}

fn source_from_row(row: &Row<'_>) -> rusqlite::Result<SourceReference> {
    Ok(SourceReference {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        tool_event_id: row.get(2)?,
        title: row.get(3)?,
        url: row.get(4)?,
        publisher: row.get(5)?,
        retrieved_at: row.get(6)?,
        excerpt: row.get(7)?,
    })
}

impl AssistantStore {
    pub fn open(path: impl AsRef<Path>) -> AssistantResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS assistant_conversations (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS assistant_messages (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id),
                 role TEXT NOT NULL CHECK (role IN ('system','user','assistant','tool')),
                 content TEXT NOT NULL,
                 provider TEXT,
                 model TEXT,
                 created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS assistant_messages_conversation
                 ON assistant_messages(conversation_id, sequence);
             CREATE TABLE IF NOT EXISTS assistant_tasks (
                 id TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id),
                 kind TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (status IN ('pending','running','waiting','completed','failed','interrupted','cancelled')),
                 checkpoint_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS assistant_tasks_conversation
                 ON assistant_tasks(conversation_id, created_at);
             CREATE TABLE IF NOT EXISTS assistant_tool_events (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id),
                 task_id TEXT REFERENCES assistant_tasks(id),
                 tool_name TEXT NOT NULL,
                 input_json TEXT NOT NULL,
                 output_json TEXT,
                 status TEXT NOT NULL CHECK (status IN ('running','completed','failed','cancelled')),
                 started_at TEXT NOT NULL,
                 completed_at TEXT
             );
             CREATE INDEX IF NOT EXISTS assistant_tool_events_conversation
                 ON assistant_tool_events(conversation_id, sequence);
             CREATE TABLE IF NOT EXISTS assistant_sources (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id),
                 tool_event_id TEXT REFERENCES assistant_tool_events(id),
                 title TEXT NOT NULL,
                 url TEXT NOT NULL,
                 publisher TEXT,
                 retrieved_at TEXT NOT NULL,
                 excerpt TEXT
             );
             CREATE INDEX IF NOT EXISTS assistant_sources_conversation
                 ON assistant_sources(conversation_id, sequence);",
        )?;
        Ok(Self { connection })
    }

    fn ensure_conversation(&self, conversation_id: &str) -> AssistantResult<()> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_conversations WHERE id = ?1)",
            [conversation_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(AssistantError::NotFound("Conversa não encontrada.".into()));
        }
        Ok(())
    }

    pub fn list_conversations(&self) -> AssistantResult<Vec<Conversation>> {
        let mut statement = self.connection.prepare(
            "SELECT id,title,created_at,updated_at FROM assistant_conversations ORDER BY updated_at DESC,rowid DESC",
        )?;
        let rows = statement.query_map([], conversation_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_conversation(&self, title: &str) -> AssistantResult<Conversation> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err(AssistantError::Validation(
                "O título deve ter entre 1 e 200 caracteres.".into(),
            ));
        }
        let now = timestamp();
        let conversation = Conversation {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.connection.execute(
            "INSERT INTO assistant_conversations(id,title,created_at,updated_at) VALUES (?1,?2,?3,?4)",
            params![conversation.id,conversation.title,conversation.created_at,conversation.updated_at],
        )?;
        Ok(conversation)
    }

    pub fn messages(&self, conversation_id: &str) -> AssistantResult<Vec<Message>> {
        self.ensure_conversation(conversation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id,conversation_id,role,content,provider,model,created_at FROM assistant_messages WHERE conversation_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([conversation_id], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> AssistantResult<Message> {
        self.ensure_conversation(conversation_id)?;
        if !["system", "user", "assistant", "tool"].contains(&role) {
            return Err(AssistantError::Validation(
                "Papel de mensagem inválido.".into(),
            ));
        }
        if content.trim().is_empty() {
            return Err(AssistantError::Validation("A mensagem está vazia.".into()));
        }
        let message = Message {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: role.into(),
            content: content.into(),
            provider: provider.map(str::to_owned),
            model: model.map(str::to_owned),
            created_at: timestamp(),
        };
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO assistant_messages(id,conversation_id,role,content,provider,model,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![message.id,message.conversation_id,message.role,message.content,message.provider,message.model,message.created_at],
        )?;
        transaction.execute(
            "UPDATE assistant_conversations SET updated_at = ?2 WHERE id = ?1",
            params![conversation_id, message.created_at],
        )?;
        transaction.commit()?;
        Ok(message)
    }

    pub fn build_context(
        &self,
        conversation_id: &str,
        max_chars: usize,
    ) -> AssistantResult<ContextBundle> {
        let messages = self.messages(conversation_id)?;
        let instruction_chars: usize = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.chars().count())
            .sum();
        if instruction_chars > max_chars {
            return Err(AssistantError::Validation(format!(
                "O limite de {max_chars} caracteres não comporta as instruções obrigatórias ({instruction_chars})."
            )));
        }
        Ok(context::build(conversation_id, messages, max_chars))
    }

    pub fn create_task(
        &self,
        conversation_id: &str,
        kind: &str,
    ) -> AssistantResult<TaskCheckpoint> {
        self.ensure_conversation(conversation_id)?;
        if kind.trim().is_empty() {
            return Err(AssistantError::Validation(
                "Informe o tipo da tarefa.".into(),
            ));
        }
        let now = timestamp();
        let task = TaskCheckpoint {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            kind: kind.into(),
            status: "pending".into(),
            checkpoint: Value::Object(Default::default()),
            created_at: now.clone(),
            updated_at: now,
        };
        self.connection.execute(
            "INSERT INTO assistant_tasks(id,conversation_id,kind,status,checkpoint_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![task.id,task.conversation_id,task.kind,task.status,task.checkpoint.to_string(),task.created_at,task.updated_at],
        )?;
        Ok(task)
    }

    /// Save the user request and its execution checkpoint atomically. The runner
    /// may then exit at any instruction without losing the requested turn.
    pub fn start_chat_task(
        &self,
        conversation_id: &str,
        content: &str,
        provider: &str,
        model: Option<&str>,
    ) -> AssistantResult<TaskCheckpoint> {
        self.ensure_conversation(conversation_id)?;
        if content.trim().is_empty() {
            return Err(AssistantError::Validation("A mensagem está vazia.".into()));
        }
        let now = timestamp();
        let task = TaskCheckpoint {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            kind: "local-cli-chat".into(),
            status: "pending".into(),
            checkpoint: Value::Object(Default::default()),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("INSERT INTO assistant_messages(id,conversation_id,role,content,provider,model,created_at) VALUES (?1,?2,'user',?3,?4,?5,?6)",params![Uuid::new_v4().to_string(),conversation_id,content,provider,model,now])?;
        transaction.execute("INSERT INTO assistant_tasks(id,conversation_id,kind,status,checkpoint_json,created_at,updated_at) VALUES (?1,?2,?3,'pending','{}',?4,?4)",params![task.id,conversation_id,task.kind,now])?;
        transaction.execute(
            "UPDATE assistant_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation_id, now],
        )?;
        transaction.commit()?;
        Ok(task)
    }

    /// A final response and the completed checkpoint are committed together;
    /// retries cannot duplicate the final assistant message.
    pub fn complete_chat_task(
        &self,
        task_id: &str,
        content: &str,
        provider: &str,
        model: Option<&str>,
        checkpoint: Value,
    ) -> AssistantResult<()> {
        if content.trim().is_empty() {
            return Err(AssistantError::Validation("Resposta vazia.".into()));
        }
        let transaction = self.connection.unchecked_transaction()?;
        let conversation_id:String=transaction.query_row("SELECT conversation_id FROM assistant_tasks WHERE id=?1 AND kind='local-cli-chat' AND status IN ('pending','running')",[task_id],|row|row.get(0))?;
        let now = timestamp();
        transaction.execute("INSERT INTO assistant_messages(id,conversation_id,role,content,provider,model,created_at) VALUES (?1,?2,'assistant',?3,?4,?5,?6)",params![Uuid::new_v4().to_string(),conversation_id,content,provider,model,now])?;
        transaction.execute("UPDATE assistant_tasks SET status='completed',checkpoint_json=?2,updated_at=?3 WHERE id=?1",params![task_id,checkpoint.to_string(),now])?;
        transaction.execute(
            "UPDATE assistant_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation_id, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn tasks(&self, conversation_id: &str) -> AssistantResult<Vec<TaskCheckpoint>> {
        self.ensure_conversation(conversation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id,conversation_id,kind,status,checkpoint_json,created_at,updated_at FROM assistant_tasks WHERE conversation_id = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map([conversation_id], task_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_checkpoint(
        &self,
        task_id: &str,
        status: &str,
        checkpoint: Value,
    ) -> AssistantResult<TaskCheckpoint> {
        if ![
            "pending",
            "running",
            "waiting",
            "completed",
            "failed",
            "interrupted",
            "cancelled",
        ]
        .contains(&status)
        {
            return Err(AssistantError::Validation(
                "Estado de tarefa inválido.".into(),
            ));
        }
        let updated = self.connection.execute(
            "UPDATE assistant_tasks SET status = ?2,checkpoint_json = ?3,updated_at = ?4 WHERE id = ?1 AND status IN ('pending','running','waiting')",
            params![task_id,status,checkpoint.to_string(),timestamp()],
        )?;
        if updated == 0 {
            return Err(AssistantError::Validation(
                "Tarefa inexistente ou já finalizada.".into(),
            ));
        }
        Ok(self.connection.query_row(
            "SELECT id,conversation_id,kind,status,checkpoint_json,created_at,updated_at FROM assistant_tasks WHERE id = ?1",
            [task_id], task_from_row,
        )?)
    }

    pub fn begin_tool_event(
        &self,
        conversation_id: &str,
        task_id: Option<&str>,
        tool_name: &str,
        input: Value,
    ) -> AssistantResult<ToolEvent> {
        self.ensure_conversation(conversation_id)?;
        if tool_name.trim().is_empty() {
            return Err(AssistantError::Validation(
                "Informe o nome da ferramenta.".into(),
            ));
        }
        if let Some(task_id) = task_id {
            let belongs: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_tasks WHERE id = ?1 AND conversation_id = ?2)",
                params![task_id,conversation_id], |row| row.get(0),
            )?;
            if !belongs {
                return Err(AssistantError::Validation(
                    "A tarefa não pertence à conversa.".into(),
                ));
            }
        }
        let event = ToolEvent {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            task_id: task_id.map(str::to_owned),
            tool_name: tool_name.into(),
            input,
            output: None,
            status: "running".into(),
            started_at: timestamp(),
            completed_at: None,
        };
        self.connection.execute(
            "INSERT INTO assistant_tool_events(id,conversation_id,task_id,tool_name,input_json,status,started_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![event.id,event.conversation_id,event.task_id,event.tool_name,event.input.to_string(),event.status,event.started_at],
        )?;
        Ok(event)
    }

    /// Terminal results are immutable: retrying a runner must not overwrite a
    /// completed tool call and erase evidence of an earlier execution.
    pub fn finish_tool_event(
        &self,
        event_id: &str,
        status: &str,
        output: Value,
    ) -> AssistantResult<ToolEvent> {
        if !["completed", "failed", "cancelled"].contains(&status) {
            return Err(AssistantError::Validation(
                "Estado final de ferramenta inválido.".into(),
            ));
        }
        let updated = self.connection.execute(
            "UPDATE assistant_tool_events SET status = ?2,output_json = ?3,completed_at = ?4 WHERE id = ?1 AND status = 'running'",
            params![event_id,status,output.to_string(),timestamp()],
        )?;
        if updated == 0 {
            return Err(AssistantError::Validation(
                "Execução inexistente ou já finalizada.".into(),
            ));
        }
        Ok(self.connection.query_row(
            "SELECT id,conversation_id,task_id,tool_name,input_json,output_json,status,started_at,completed_at FROM assistant_tool_events WHERE id = ?1",
            [event_id], tool_event_from_row,
        )?)
    }

    pub fn tool_events(&self, conversation_id: &str) -> AssistantResult<Vec<ToolEvent>> {
        self.ensure_conversation(conversation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id,conversation_id,task_id,tool_name,input_json,output_json,status,started_at,completed_at FROM assistant_tool_events WHERE conversation_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([conversation_id], tool_event_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_source(
        &self,
        conversation_id: &str,
        tool_event_id: Option<&str>,
        title: &str,
        url: &str,
        publisher: Option<&str>,
        excerpt: Option<&str>,
    ) -> AssistantResult<SourceReference> {
        self.ensure_conversation(conversation_id)?;
        if title.trim().is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(AssistantError::Validation(
                "A fonte exige título e URL HTTP(S).".into(),
            ));
        }
        if let Some(event_id) = tool_event_id {
            let event_conversation: Option<String> = self
                .connection
                .query_row(
                    "SELECT conversation_id FROM assistant_tool_events WHERE id = ?1",
                    [event_id],
                    |row| row.get(0),
                )
                .optional()?;
            if event_conversation.as_deref() != Some(conversation_id) {
                return Err(AssistantError::Validation(
                    "A execução não pertence à conversa.".into(),
                ));
            }
        }
        let source = SourceReference {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            tool_event_id: tool_event_id.map(str::to_owned),
            title: title.into(),
            url: url.into(),
            publisher: publisher.map(str::to_owned),
            retrieved_at: timestamp(),
            excerpt: excerpt.map(str::to_owned),
        };
        self.connection.execute(
            "INSERT INTO assistant_sources(id,conversation_id,tool_event_id,title,url,publisher,retrieved_at,excerpt) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![source.id,source.conversation_id,source.tool_event_id,source.title,source.url,source.publisher,source.retrieved_at,source.excerpt],
        )?;
        Ok(source)
    }

    pub fn sources(&self, conversation_id: &str) -> AssistantResult<Vec<SourceReference>> {
        self.ensure_conversation(conversation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id,conversation_id,tool_event_id,title,url,publisher,retrieved_at,excerpt FROM assistant_sources WHERE conversation_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([conversation_id], source_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
