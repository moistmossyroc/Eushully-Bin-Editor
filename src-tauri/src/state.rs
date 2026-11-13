use crate::bin_format::parser::ParsedBin;
use crate::claude::client::ApiProvider;
use crate::claude::session::SessionControl;
use crate::editor::bridge::BridgeTemplate;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::watch;

/// Per-tab data stored on the Rust side.
pub struct TabData {
    /// The parsed BIN file (all instructions)
    pub parsed_bin: Option<ParsedBin>,
    /// Raw bytes of the original BIN file (needed for export reference)
    pub raw_bin_data: Option<Vec<u8>>,
    /// Path to the loaded BIN file
    pub bin_path: Option<String>,
    /// Control channel sender for the translation session
    pub translation_control_tx: Option<watch::Sender<SessionControl>>,
    /// Cached bridge template for auto-textbox splitting. Populated once on
    /// BIN load; `None` means detection hasn't run yet.
    pub bridge_template: Option<BridgeTemplate>,
}

impl TabData {
    pub fn new() -> Self {
        Self {
            parsed_bin: None,
            raw_bin_data: None,
            bin_path: None,
            translation_control_tx: None,
            bridge_template: None,
        }
    }
}

/// Application state managed by Tauri.
pub struct AppState {
    /// Per-tab state keyed by tab ID
    pub tabs: Mutex<HashMap<String, TabData>>,
    /// Stored API key (loaded from OS keyring on startup)
    pub api_key: Mutex<Option<String>>,
    /// Which API format to use (Anthropic or OpenAI-compatible)
    pub api_provider: Mutex<ApiProvider>,
    /// Custom endpoint URL for OpenAI-compatible providers
    pub custom_endpoint: Mutex<Option<String>>,
    /// Per-game speaker tables (name-table scripts, talk-key owners) keyed by game folder
    /// + a change stamp (`engine::game_files_stamp`). Every scene of a game shares one
    /// parse of CNINIT/CHINIT/INIT2/… (CHINIT alone can be 80k+ instructions).
    pub speaker_tables: Mutex<HashMap<String, std::sync::Arc<crate::engine::GameSpeakerTables>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tabs: Mutex::new(HashMap::new()),
            api_key: Mutex::new(None),
            api_provider: Mutex::new(ApiProvider::default()),
            custom_endpoint: Mutex::new(None),
            speaker_tables: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a TabData entry for the given tab ID.
    pub fn ensure_tab(&self, tab_id: &str) -> () {
        let mut tabs = self.tabs.lock().unwrap();
        tabs.entry(tab_id.to_string()).or_insert_with(TabData::new);
    }

    /// Remove a tab's data, freeing memory.
    pub fn remove_tab(&self, tab_id: &str) {
        let mut tabs = self.tabs.lock().unwrap();
        tabs.remove(tab_id);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
