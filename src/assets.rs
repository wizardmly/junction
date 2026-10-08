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
        Lock,
        CircleQuestionMark,
        Pencil,
        ArrowLeft,
        ArrowRight,
        List,
        WandSparkles,
        Link2, Scissors, Clipboard, Columns2, TextAlignStart, EllipsisVertical, ArrowLeftRight, Ban,
        Funnel, Pin, SquareArrowDownLeft, TextSearch, FileSearch, Zap, Keyboard, ListTree, ListOrdered, SquareSplitVertical, SquareFunction, Braces, Variable, ClockArrowUp
    ]
);

/// Our extra icons first, then the component library's defaults.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
