//! UI language: English and Traditional Chinese, switched at runtime with `L`.
//! Every UI string lives in one table keyed by [`Text`]; data from the daemon
//! (titles, questions, summaries) is shown as it arrives.
//!
//! Must NOT: translate program identifiers (task ids, branch names, commands).

/// Languages the TUI can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    ZhTw,
}

impl Language {
    /// The language after pressing `L`.
    pub fn toggled(self) -> Language {
        match self {
            Language::En => Language::ZhTw,
            Language::ZhTw => Language::En,
        }
    }

    pub fn parse(s: &str) -> Option<Language> {
        match s {
            "en" => Some(Language::En),
            "zh-TW" | "zh-tw" => Some(Language::ZhTw),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::ZhTw => "zh-TW",
        }
    }

    pub fn tr(self, text: Text) -> &'static str {
        let (en, zh) = strings(text);
        match self {
            Language::En => en,
            Language::ZhTw => zh,
        }
    }

    /// `tr` with each `{}` replaced by the next argument.
    pub fn fmt(self, text: Text, args: &[&str]) -> String {
        let mut out = String::new();
        let mut args = args.iter();
        let mut parts = self.tr(text).split("{}").peekable();
        while let Some(part) = parts.next() {
            out.push_str(part);
            if parts.peek().is_some() {
                out.push_str(args.next().copied().unwrap_or(""));
            }
        }
        out
    }
}

/// Every UI string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    CrumbNeedsYou,
    CrumbTerminal,
    NeedsYouN,
    NeedsYouOpen,
    NothingNeedsYou,
    BackToHome,
    NoActiveGoals,
    New,
    NoTeam,
    Working,
    Idle,
    NeedsYouState,
    Stuck,
    Unknown,
    StatusDone,
    StatusWaitingYou,
    StatusRunning,
    TabGoals,
    TabAgents,
    TabPipeline,
    TeamNoGoals,
    TeamNoAgents,
    PipelineDone,
    ColState,
    ColName,
    ColBackend,
    ColTask,
    AskFrom,
    IfIgnored,
    RecapGoal,
    RecapDecided,
    RecapAsking,
    RecapNext,
    EntryAnswer,
    EntryResolution,
    AnswerOwnWords,
    NoAction,
    AnswerSent,
    AnswerPrompt,
    Rejected,
    TaskMeta,
    NoRepo,
    TaskStatus,
    Stages,
    StageDone,
    StageRunning,
    StageNotStarted,
    RecentEvents,
    NoneYet,
    AgentMeta,
    AgentStateLine,
    CurrentTask,
    NoCurrentTask,
    ViewTerminal,
    TerminalTitle,
    NoOutputFor,
    NoAgentRow,
    FinderTitle,
    FinderCount,
    FinderNone,
    Disconnected,
    DiscReason,
    DiscRetrying,
    DiscAttempt,
    DiscSafe,
    Reconnected,
    TooSmall,
    MoreBelow,
    KeyMove,
    KeyScroll,
    KeyOpen,
    KeyChoose,
    KeyOption,
    KeyAnswer,
    KeyTerminal,
    KeyNeedsYou,
    KeyFind,
    KeyTabs,
    KeyBack,
    KeyHome,
    KeyQuit,
    HelpFinder,
    HelpDisconnected,
    LangKey,
}

fn strings(text: Text) -> (&'static str, &'static str) {
    use Text::*;
    match text {
        CrumbNeedsYou => ("Needs you", "需要你"),
        CrumbTerminal => ("Terminal", "終端"),
        NeedsYouN => ("Needs you · {}", "需要你 · {}"),
        NeedsYouOpen => ("Needs you · {} open", "需要你 · {} 項待處理"),
        NothingNeedsYou => ("Nothing needs you right now.", "目前沒有需要你處理的事項。"),
        BackToHome => ("[ Back to Home ]", "[ 返回首頁 ]"),
        NoActiveGoals => ("no active goals", "沒有進行中的目標"),
        New => ("new", "新"),
        NoTeam => ("no team", "無 team"),
        Working => ("working", "工作中"),
        Idle => ("idle", "閒置"),
        NeedsYouState => ("needs you", "需要你"),
        Stuck => ("stuck", "卡住"),
        Unknown => ("unknown", "未知"),
        StatusDone => ("done", "已完成"),
        StatusWaitingYou => ("waiting for you", "等待你"),
        StatusRunning => ("running: {}", "執行中：{}"),
        TabGoals => ("1 Goals", "1 目標"),
        TabAgents => ("2 Agents", "2 Agent"),
        TabPipeline => ("3 Pipeline", "3 流水線"),
        TeamNoGoals => ("No goals for this team.", "此 team 沒有目標。"),
        TeamNoAgents => ("No agents in this team.", "此 team 沒有 agent。"),
        PipelineDone => ("done", "完成"),
        ColState => ("State", "狀態"),
        ColName => ("Name", "名稱"),
        ColBackend => ("Backend", "backend"),
        ColTask => ("Current task", "目前任務"),
        AskFrom => ("From {} · task {} · team {}", "來自 {} · 任務 {} · team {}"),
        IfIgnored => ("If you leave it: {}", "不處理的話：{}"),
        RecapGoal => ("Goal: {}", "目標：{}"),
        RecapDecided => ("Decided so far: {}", "目前已決定：{}"),
        RecapAsking => ("Asking: {}", "在問：{}"),
        RecapNext => ("Next: {}", "接下來：{}"),
        EntryAnswer => ("you ({}): {}", "你（{}）：{}"),
        EntryResolution => ("resolved: {}", "已有結論：{}"),
        AnswerOwnWords => ("[a] Answer in your own words", "[a] 用自己的話回答"),
        NoAction => (
            "No operator action for this item in client protocol v1 yet (gate 8).",
            "client protocol v1 還沒有處理這一項的操作（第 8 施工關）。",
        ),
        AnswerSent => ("Answer sent to {}: {}", "已送出對 {} 的回答：{}"),
        AnswerPrompt => (
            "Answer {}: {}▏  Enter send · Esc cancel",
            "回答 {}：{}▏  Enter 送出 · Esc 取消",
        ),
        Rejected => ("The daemon rejected it: {}", "daemon 拒絕：{}"),
        TaskMeta => (
            "team {} · repo {} · {} · agent: {}",
            "team {} · repo {} · {} · agent：{}",
        ),
        NoRepo => ("none", "無"),
        TaskStatus => (
            "Status: {} · {}/{} stages done",
            "狀態：{} · 已完成 {}/{} 個關卡",
        ),
        Stages => ("Stages", "關卡"),
        StageDone => ("done", "完成"),
        StageRunning => ("running", "執行中"),
        StageNotStarted => ("not started", "尚未開始"),
        RecentEvents => ("Recent events", "最近事件"),
        NoneYet => ("None.", "無。"),
        AgentMeta => ("team {} · backend {}", "team {} · backend {}"),
        AgentStateLine => ("State: {}", "狀態：{}"),
        CurrentTask => ("Current task: {} {}", "目前任務：{} {}"),
        NoCurrentTask => ("Current task: none", "目前任務：無"),
        ViewTerminal => ("[t] View terminal", "[t] 看終端"),
        TerminalTitle => (
            "Terminal of {} · read-only snapshot",
            "{} 的終端 · 唯讀快照",
        ),
        NoOutputFor => ("No terminal output for {}.", "{} 沒有終端輸出。"),
        NoAgentRow => ("This row has no agent.", "這一列沒有 agent。"),
        FinderTitle => ("Find · agents, tasks, teams", "搜尋 · agent、任務、team"),
        FinderCount => ("{} found", "{} 筆結果"),
        FinderNone => ("No matches.", "找不到。"),
        Disconnected => ("Daemon disconnected", "daemon 已斷線"),
        DiscReason => (
            "Lost the connection to the daemon: {}",
            "與 daemon 的連線中斷：{}",
        ),
        DiscRetrying => ("Reconnecting automatically…", "正在自動重新連線…"),
        DiscAttempt => (
            "Reconnect attempt {} failed: {}",
            "第 {} 次重新連線失敗：{}",
        ),
        DiscSafe => (
            "The daemon keeps all state; this view comes back when it reconnects.",
            "狀態都在 daemon；重新連上後畫面就會回來。",
        ),
        Reconnected => ("Reconnected to the daemon.", "已重新連上 daemon。"),
        TooSmall => (
            "Terminal too small: {}×{} (need at least 70×20).",
            "終端機太小：{}×{}（至少要 70×20）。",
        ),
        MoreBelow => ("↓ {} more lines below", "↓ 下方還有 {} 行"),
        KeyMove => ("↑↓ move", "↑↓ 移動"),
        KeyScroll => ("↑↓ scroll", "↑↓ 捲動"),
        KeyOpen => ("→/Enter open", "→/Enter 開啟"),
        KeyChoose => ("→/Enter choose", "→/Enter 選擇"),
        KeyOption => ("1-9 option", "1-9 選項"),
        KeyAnswer => ("a answer", "a 回答"),
        KeyTerminal => ("t terminal", "t 終端"),
        KeyNeedsYou => ("! needs you", "! 需要你"),
        KeyFind => ("/ find", "/ 搜尋"),
        KeyTabs => ("1/2/3 or Tab tabs", "1/2/3 或 Tab 切換分頁"),
        KeyBack => ("←/Esc back", "←/Esc 返回"),
        KeyHome => ("h home", "h 首頁"),
        KeyQuit => ("q quit", "q 離開"),
        HelpFinder => (
            "type to filter · ↑↓ move · →/Enter open · ←/Esc close",
            "輸入以篩選 · ↑↓ 移動 · →/Enter 開啟 · ←/Esc 關閉",
        ),
        HelpDisconnected => ("r retry now · {} · q quit", "r 立即重試 · {} · q 離開"),
        LangKey => ("L 中文", "L English"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_cycles_between_the_two_languages() {
        assert_eq!(Language::En.toggled(), Language::ZhTw);
        assert_eq!(Language::En.toggled().toggled(), Language::En);
    }

    #[test]
    fn fmt_fills_placeholders_in_order() {
        assert_eq!(
            Language::En.fmt(Text::AskFrom, &["dev-2", "T-45", "archfix"]),
            "From dev-2 · task T-45 · team archfix"
        );
        assert_eq!(Language::ZhTw.fmt(Text::NeedsYouN, &["2"]), "需要你 · 2");
    }
}
