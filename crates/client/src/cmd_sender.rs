use tokio::sync::mpsc;
use war3_protocol::messages::ClientMessage;

/// Newtype wrapper for the cmd channel sender 強制使用 `send_or_warn`，
/// 避免新增 call site 時直接 `.send()` 繞過 logging。
///
/// 演進：PR #28 加了 `try_send_cmd` free function + `#[must_use]`（解 #23
/// silent-drop bug），但 `mpsc::UnboundedSender<ClientMessage>` 仍可被
/// bypass。本 newtype 收緊 type system 把它變成不可能（#32）。
#[derive(Clone)]
pub struct CmdSender(mpsc::UnboundedSender<ClientMessage>);

impl CmdSender {
    pub(crate) fn new(tx: mpsc::UnboundedSender<ClientMessage>) -> Self {
        Self(tx)
    }

    /// 送 cmd 到 background network task。失敗時 warn log，回傳 true = 成功。
    ///
    /// `action_label` 是**使用者可見的繁體中文動詞片語**（e.g. "建立房間"、
    /// "加入"、"關閉房間"），會出現在 UI log panel 為
    /// `"{action_label} 未送出：背景任務已中斷（...）"`。不要傳英文 dev string——
    /// 使用者直接看到。
    ///
    /// 回傳 bool 必須處理：若忽略則退化為 silent drop（#23 修復前的行為）。
    #[must_use = "send_or_warn 失敗時呼叫端需要清除 pending UI 狀態，否則使用者會卡在 loading"]
    pub fn send_or_warn(&self, msg: ClientMessage, action_label: &str) -> bool {
        if let Err(e) = self.0.send(msg) {
            tracing::warn!(
                verbosity = "concise",
                "{action_label} 未送出：背景任務已中斷（{e}）"
            );
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_or_warn_returns_true_when_receiver_alive() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = CmdSender::new(tx);
        let ok = sender.send_or_warn(ClientMessage::CloseRoom, "關閉房間");
        assert!(ok);
        assert!(matches!(rx.try_recv(), Ok(ClientMessage::CloseRoom)));
    }

    #[test]
    fn send_or_warn_returns_false_when_receiver_dropped() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sender = CmdSender::new(tx);
        drop(rx);
        let ok = sender.send_or_warn(ClientMessage::CloseRoom, "關閉房間");
        assert!(!ok);
    }
}
