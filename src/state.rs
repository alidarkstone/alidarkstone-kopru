//! Arayüzün okuduğu anlık görüntü ve arayüzün ürettiği eylemler.
//! Arayüz (src/ui) yalnızca bu dosyadaki türleri bilir; ağ/özellik koduna doğrudan dokunmaz.

use crate::msg::Edge;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum LinkPhase {
    Idle,
    Connecting,
    Checking,
    /// sas: 6 hane; mine/theirs: bu taraf / karşı taraf onayladı mı
    Pairing { sas: String, mine: Option<bool>, theirs: Option<bool> },
    Connected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerView {
    pub id: String,
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrustedView {
    pub id: String,
    pub name: String,
    pub since: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Local,
    /// bu bilgisayarın imleci diğerinde
    Remote,
    /// diğer bilgisayar bunu kontrol ediyor
    Controlled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputView {
    pub available: bool,
    /// macOS: Erişilebilirlik izni gerekli ama verilmemiş
    pub needs_permission: bool,
    pub enabled: bool,
    pub edge: Edge,
    pub mode: InputMode,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDir {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Waiting,
    Sending,
    Sent,
    Offered,
    Receiving,
    Received,
    Rejected,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransferView {
    pub id: String,
    pub dir: TransferDir,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub status: TransferStatus,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub my_id: String,
    pub my_name: String,
    pub phase: LinkPhase,
    pub peer: Option<PeerView>,
    pub last_error: String,
    pub devices: Vec<DeviceView>,
    pub trusted: Vec<TrustedView>,
    pub clipboard_enabled: bool,
    pub input: InputView,
    pub transfers: Vec<TransferView>,
    pub download_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SetName(String),
    Connect(String),           // cihaz kimliği (keşif listesinden)
    ConnectAddress(String),    // "192.168.1.20" veya "host:port"
    Disconnect,
    ConfirmPairing(bool),
    Forget(String),
    SetClipboard(bool),
    SetInput { enabled: bool, edge: Edge },
    PickFiles,                 // dosya seçme penceresi aç ve gönder
    SendPaths(Vec<PathBuf>),   // sürükle-bırak
    FileAccept(String),
    FileReject(String),
    FileCancel(String),
    FileShow(String),
    OpenDownloads,
    ChooseDownloadDir,
    OpenAccessibilitySettings, // macOS
}

impl UiState {
    /// Arayüz geliştirme ve ekran görüntüsü için örnek veri.
    pub fn demo() -> UiState {
        UiState {
            my_id: "4d21 81e7 e8d6 040e".into(),
            my_name: "Masaüstü".into(),
            phase: LinkPhase::Connected,
            peer: Some(PeerView { id: "9a0c11d2e3f40516".into(), name: "MacBook Air".into(), address: "192.168.1.24".into() }),
            last_error: String::new(),
            devices: vec![
                DeviceView { id: "9a0c11d2e3f40516".into(), name: "MacBook Air".into(), address: "192.168.1.24".into(), port: 47910, trusted: true },
                DeviceView { id: "1b2c3d4e5f607182".into(), name: "Ofis-PC".into(), address: "192.168.1.31".into(), port: 47910, trusted: false },
            ],
            trusted: vec![TrustedView { id: "9a0c11d2e3f40516".into(), name: "MacBook Air".into(), since: "2026-11-01".into() }],
            clipboard_enabled: true,
            input: InputView { available: true, needs_permission: false, enabled: true, edge: Edge::Right, mode: InputMode::Local, error: String::new() },
            transfers: vec![
                TransferView { id: "a1".into(), dir: TransferDir::In, name: "Sunum.key".into(), size: 48_000_000, done: 31_000_000, status: TransferStatus::Receiving, path: None },
                TransferView { id: "a2".into(), dir: TransferDir::Out, name: "fatura.pdf".into(), size: 220_000, done: 220_000, status: TransferStatus::Sent, path: None },
                TransferView { id: "a3".into(), dir: TransferDir::In, name: "foto.heic".into(), size: 3_400_000, done: 0, status: TransferStatus::Offered, path: None },
            ],
            download_dir: PathBuf::from("C:/Users/darkstone/Downloads/Köprü"),
        }
    }
}
