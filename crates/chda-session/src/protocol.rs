use chda_term::{
    ColorConfig, Event, Frame, KeyInput, Modifiers, MouseInput, SearchQuery, SessionOptions, Size,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

pub const PROTOCOL_VERSION: u32 = 1;
// A frame can carry Kitty's bounded image storage. JSON byte arrays are at
// most four bytes per pixel byte; the framing limit is checked before allocation.
const MAX_MESSAGE: usize = 1_400_000_000;
const MAX_REQUEST: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub session: String,
    pub run: String,
    pub server: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub pane: u64,
    pub identity: Identity,
    pub pid: Option<u32>,
    pub running: bool,
    #[serde(default)]
    pub closed: bool,
    pub title: String,
    pub cwd: Option<PathBuf>,
    pub launch: Option<chda_agents::AgentLaunchContext>,
    pub agent: Option<chda_agents::HookEvent>,
    pub exit: Option<chda_term::ExitStatus>,
    pub saved_at: Option<u64>,
    pub save_error: Option<String>,
    pub scrollback: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub instance: String,
    pub executable_version: String,
    pub layout: Option<chda_core::SavedSession>,
    pub sessions: Vec<SessionRecord>,
    pub save_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Key(KeyInput),
    Paste(String),
    Text(String),
    Bytes(Vec<u8>),
    Focus(bool),
    Resize {
        size: Size,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    Scroll {
        lines: isize,
        mods: Modifiers,
        px_x: u32,
        px_y: u32,
    },
    ScrollToBottom,
    JumpToPrompt(i32),
    JumpToLastPrompt,
    Mouse(MouseInput),
    CopySelection,
    ReadSelection(u64),
    FindLastDiagram,
    QueryCwd,
    Search(Option<SearchQuery>),
    SearchStep(bool),
    SetColors(ColorConfig),
}

impl Action {
    pub(crate) fn apply(self, session: &chda_term::Session) {
        match self {
            Self::Key(i) => session.key(i),
            Self::Paste(t) => session.paste(t),
            Self::Text(t) => session.text(t),
            Self::Bytes(b) => session.write(b),
            Self::Focus(f) => session.focus(f),
            Self::Resize {
                size,
                cell_width_px,
                cell_height_px,
            } => session.resize(size, cell_width_px, cell_height_px),
            Self::Scroll {
                lines,
                mods,
                px_x,
                px_y,
            } => session.scroll(lines, mods, px_x, px_y),
            Self::ScrollToBottom => session.scroll_to_bottom(),
            Self::JumpToPrompt(d) => session.jump_to_prompt(d),
            Self::JumpToLastPrompt => session.jump_to_last_prompt(),
            Self::Mouse(i) => session.mouse(i),
            Self::CopySelection => session.copy_selection(),
            Self::ReadSelection(r) => session.read_selection(r),
            Self::FindLastDiagram => session.find_last_diagram(),
            Self::QueryCwd => session.query_cwd(),
            Self::Search(q) => session.search(q),
            Self::SearchStep(o) => session.search_step(o),
            Self::SetColors(c) => session.set_colors(c),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Envelope {
    pub version: u32,
    pub request: Request,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    Hello,
    List,
    ReserveIds,
    Create {
        pane: u64,
        options: SessionOptions,
    },
    Attach {
        identity: Identity,
    },
    Input {
        identity: Identity,
        action: Action,
    },
    Poll {
        identity: Identity,
        generation: u64,
        sequence: u64,
    },
    Detach,
    Terminate {
        identity: Identity,
    },
    Resume {
        identity: Identity,
        options: SessionOptions,
    },
    Checkpoint {
        layout: Option<chda_core::SavedSession>,
    },
    Output {
        identity: Identity,
    },
    HookEvents {
        sequence: u64,
    },
    GuiReply {
        id: u64,
        reply: chda_agents::control::Reply,
    },
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Response {
    IdRange(u64),
    State(State),
    Attached {
        record: SessionRecord,
        frame: Box<Frame>,
        sequence: u64,
    },
    Update {
        record: SessionRecord,
        frame: Option<Box<Frame>>,
        events: Vec<Event>,
        sequence: u64,
    },
    Hooks {
        events: Vec<chda_agents::HookEvent>,
        quotas: Vec<chda_agents::quota::QuotaSnapshot>,
        requests: Vec<(u64, chda_agents::control::Request)>,
        sequence: u64,
    },
    Output(String),
    Ok,
    Error(String),
}

pub(crate) fn write<T: Serialize>(stream: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::other("Session message exceeds its size limit"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}
pub(crate) fn read<T: serde::de::DeserializeOwned>(
    stream: &mut impl Read,
    request: bool,
) -> io::Result<T> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > if request { MAX_REQUEST } else { MAX_MESSAGE } {
        return Err(io::Error::other("Session message exceeds its size limit"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_declared_oversized_message_is_rejected_before_reading_its_body() {
        let mut bytes = &u32::MAX.to_be_bytes()[..];
        assert!(read::<Envelope>(&mut bytes, true).is_err());
        assert!(bytes.is_empty());
    }
}
