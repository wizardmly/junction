use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

// Icons the Git client needs beyond the component library's default set.
gpui_kit::assets::icon_assets!(
    GitIcons,
    [
        GitBranch,
        GitBranchPlus,
        GitCommitVertical,
        GitCompare,
        GitFork,
        GitGraph,
        GitMerge,
        GitMergeConflict,
        GitPullRequest,
        FolderGit2,
        Tag,
        Tags,
        Download,
        Upload,
        CloudDownload,
        Check,
        CheckCheck,
        Undo2,
        Clock,
        ListFilter,
        Terminal,
        FileDiff,
        FolderTree,
        Settings,
        Menu,
        Eye,
        Calendar,
        UserRound,
        Folder,
        FolderClosed,
        FolderOpen,
        File,
        FileCode,
        CircleDot,
        ArrowDownToLine,
        ArrowUpFromLine,
        RefreshCw,
        Archive,
        Layers,
        Library,
        Bot,
        FileArchive,
        SquareTerminal,
        Image,
        CodeXml,
        Copy,
        Rows3,
        PanelLeft,
        PanelBottom,
        Star,
        Plus,
        Minus,
        X,
        Search,
        ChevronDown,
        ChevronRight,
        ChevronUp,
        Circle,
        FoldVertical,
        Hash,
        Settings2,
        ChevronsDownUp,
        ChevronsUpDown,
        StarOff,
        MessageSquare,
        Globe,
        ChevronLeft,
        ChevronsRight,
        ChevronsLeft,
        ArrowRightToLine,
        ArrowLeftToLine,
        Lock,
        CircleQuestionMark,
        Pencil,
        ArrowLeft,
        ArrowRight,
        List,
        WandSparkles,
        Link2, Scissors, Clipboard, Columns2, TextAlignStart, EllipsisVertical, ArrowLeftRight, Ban,
        Funnel, Pin, SquareArrowDownLeft, TextSearch, FileSearch, Zap, Keyboard, ListTree, ListOrdered, SquareSplitVertical, SquareFunction, Braces, Variable, ClockArrowUp, Crosshair,
        Bell, BellDot, LoaderCircle, Regex, WholeWord, CaseSensitive, TextWrap, Play
    ]
);

/// Our extra icons first, then the component library's defaults.
pub struct AppAssets;

/// IntelliJ-style icons of our own (Project view), served as "junction/<name>.svg".
const OWN_ICONS: &[(&str, &[u8])] = &[
    ("junction/android.svg", include_bytes!("../assets/icons/android.svg")),
    ("junction/markdown.svg", include_bytes!("../assets/icons/markdown.svg")),
];

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN_ICONS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(*bytes)));
        }
        if let Some(bytes) = path.starts_with("as/").then(|| crate::ui::as_icons::load(path)).flatten() {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = GitIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(GitIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
