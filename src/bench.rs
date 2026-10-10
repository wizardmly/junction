//! Backend timings on a real repository, for performance work:
//! `JUNCTION_BENCH=/path/to/repo cargo test --release bench_backend -- --ignored --nocapture`
//! Optional: `JUNCTION_BENCH_AT=file:needle,...` (jumps), `JUNCTION_BENCH_DIFF=file`
//! (diffs that file against its version 50 commits back).

use std::path::PathBuf;
use std::time::Instant;

use crate::git::{self, GitConsole, Repository};
use crate::index;

fn rss() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status.lines().find(|l| l.starts_with("VmRSS")).unwrap_or("").split_whitespace().nth(1).map(|kb| format!("{} MB", kb.parse::<u64>().unwrap_or(0) / 1024)).unwrap_or_default()
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    println!("{label:<34} {:>10.1} ms   rss {}", t.elapsed().as_secs_f64() * 1000.0, rss());
    out
}

#[test]
#[ignore]
fn bench_backend() {
    let root = PathBuf::from(std::env::var("JUNCTION_BENCH").expect("JUNCTION_BENCH"));
    let repository = Repository::discover(&root, GitConsole::default()).unwrap();
    let filter = git::log::LogFilter::default();

    let skip_log = std::env::var("JUNCTION_BENCH_SKIP_LOG").is_ok();
    time("refs", || git::refs::RepositoryRefs::load(&repository).unwrap());
    let page = time("log first page", || git::log::load_first_page(&repository, &filter).unwrap());
    time("graph first page", || git::graph::GraphLayout::build(&page));
    if !skip_log {
    let all = time("log full", || git::log::load_log(&repository, &filter, None).unwrap());
    println!("  {} commits", all.len());
    let graph = time("graph full", || git::graph::GraphLayout::build(&all));
    time("commit rows", || git::log::CommitRows::build(&all));
    if let Ok(n) = std::env::var("JUNCTION_BENCH_CHECK_GRAPH") {
        time("graph check vs eager layout", || git::graph::reference::assert_same(&all[..all.len().min(n.parse().unwrap())]));
    }
    time("graph rows (long edges hidden)", || graph.rows(graph.len() / 2..graph.len() / 2 + 60, false));
    drop((graph, all));
    }
    time("status", || git::status::WorkingTreeStatus::load(&repository).unwrap());
    time("status again", || git::status::WorkingTreeStatus::load(&repository).unwrap());

    if let Ok(file) = std::env::var("JUNCTION_BENCH_DIFF") {
        let old = repository.run(["show", &format!("HEAD~50:{file}")]).unwrap();
        let new = std::fs::read_to_string(root.join(&file)).unwrap();
        let diff = time("diff big file", || git::diff::compute(&old, &new, git::diff::DiffOptions::default()));
        println!("  {} rows, {} changes", diff.rows.len(), diff.changes);
        let options = git::diff::DiffOptions { context: None, ..Default::default() };
        time("diff big file (no folding)", || git::diff::compute(&old, &new, options));
    }

    if std::env::var("JUNCTION_BENCH_SKIP_INDEX").is_ok() {
        return;
    }
    let cache = PathBuf::from(repository.run(["rev-parse", "--absolute-git-dir"]).unwrap().trim()).join("junction/index.bin");
    std::fs::remove_file(&cache).ok();
    let mut idx = time("index load (no cache)", || index::store::ProjectIndex::load(&root));
    time("index build", || idx.update(&|_, _| {}));
    println!("  {} files, {} symbols", idx.files.len(), idx.symbol_count());
    time("index save", || idx.save());
    println!("  cache {} MB", std::fs::metadata(&cache).map(|m| m.len() / 1_000_000).unwrap_or(0));
    drop(idx);
    let mut idx = time("index load (cache)", || index::store::ProjectIndex::load(&root));
    time("index update (unchanged)", || idx.update(&|_, _| {}));
    time("index clone", || idx.clone());
    if let Some(rel) = idx.files.keys().nth(idx.files.len() / 2).cloned() {
        time("index refresh one file", || idx.refresh_file(&rel));
    }
    time("search symbols 'sched'", || index::nav::search_symbols(&idx, "sched", false, false, 200));
    time("search symbols 'kmalloc'", || index::nav::search_symbols(&idx, "kmalloc", false, false, 200));
    time("search files 'core.c'", || index::nav::search_files(&idx, "core.c", false, 200));
    time("search files 'schcore'", || index::nav::search_files(&idx, "schcore", false, 200));
    if let Ok(spec) = std::env::var("JUNCTION_BENCH_AT") {
        for spec in spec.split(',') {
            let (file, needle) = spec.split_once(':').unwrap();
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let offset = text.find(needle).unwrap() + 1;
            let lang = index::service::CodeIndex::lang_of(file, &text).unwrap();
            let targets = time(&format!("definitions {needle}"), || index::nav::definitions(&idx, file, lang, &text, offset));
            println!("  {} targets", targets.len());
            let word = needle.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            let usages = time(&format!("usages {word}"), || index::nav::usages(&idx, &root, word, Some(lang)));
            println!("  {} usages", usages.len());
        }
    }
}

/// Where indexing time goes: parsing, symbol extraction, bridges.
#[test]
#[ignore]
fn bench_index_stages() {
    use crate::index::{bridge, lang::Lang, symbols};
    let root = PathBuf::from(std::env::var("JUNCTION_BENCH").expect("JUNCTION_BENCH"));
    let files: Vec<String> = index::store::list_files(&root).into_iter().filter(|f| Lang::from_path(f).is_some()).step_by(10).collect();
    let texts: Vec<(Lang, String, String)> = files
        .iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(root.join(f)).ok()?;
            let mut lang = Lang::from_path(f)?;
            if f.ends_with(".h") {
                lang = Lang::for_header(&text);
            }
            Some((lang, f.clone(), text))
        })
        .collect();
    println!("{} files", texts.len());
    let trees: Vec<_> = time("parse", || texts.iter().map(|(l, _, t)| symbols::parse(*l, t)).collect());
    let syms: Vec<_> = time("extract symbols", || texts.iter().zip(&trees).map(|((l, _, t), tree)| tree.as_ref().map(|tree| symbols::extract_from(*l, t, tree)).unwrap_or_default()).collect());
    time("bridges", || texts.iter().zip(&syms).map(|((l, p, t), s)| bridge::extract(*l, p, t, s).len()).sum::<usize>());
}

#[test]
#[ignore]
fn bench_symbol_search() {
    let root = PathBuf::from(std::env::var("JUNCTION_BENCH").expect("JUNCTION_BENCH"));
    let idx = time("index load (cache)", || index::store::ProjectIndex::load(&root));
    println!("  {} names", idx.names().count());
    let fuzzy = index::nav::Fuzzy::new("sched");
    let n = time("score all names", || idx.names().filter(|n| fuzzy.score(n).is_some()).count());
    println!("  {n} match");
    let n = time("iterate names", || idx.names().map(|n| n.len()).sum::<usize>());
    println!("  {n}");
    for q in ["sched", "kmalloc", "KMC", "x", "init", "spin_lock"] {
        let r = time(&format!("search symbols '{q}'"), || index::nav::search_symbols(&idx, q, false, false, 200));
        println!("  {} results", r.len());
        // The plain way: every name, every symbol.
        let mut all: Vec<(i32, usize, String)> = Vec::new();
        for name in idx.names() {
            let Some(score) = index::nav::fuzzy_score(q, name) else { continue };
            for (p, _, s) in idx.symbols_named(name) {
                let exact = if name.eq_ignore_ascii_case(q) { 50 } else { 0 };
                all.push((score + exact - s.decl as i32 * 5, s.name.len(), p.to_owned()));
            }
        }
        all.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        all.truncate(200);
        let got: Vec<(i32, usize, String)> = r.iter().map(|m| (m.score, m.target.name.len(), m.target.path.clone())).collect();
        assert_eq!(got, all, "{q}");
    }
}

/// Steps of switching a file in the diff viewer (run in this repository).
#[test]
#[ignore]
fn bench_diff_switch() {
    let root = PathBuf::from(std::env::var("JUNCTION_BENCH").expect("JUNCTION_BENCH"));
    let repository = Repository::discover(&root, GitConsole::default()).unwrap();
    let hash = repository.run(["rev-parse", "449b918"]).unwrap().trim().to_owned();
    for path in ["src/git/graph.rs", "src/ui/log_view.rs", "src/model.rs"] {
        println!("{path}");
        let rev = git::diff::Revisions::Commit { hash: hash.clone(), path: path.into(), old_path: None };
        let (old, new, _, _) = time("load_versions", || git::diff::load_versions(&repository, &rev).unwrap());
        time("compute", || git::diff::compute(&old, &new, Default::default()));
        let texts = vec![old, new];
        time("highlight_texts", || crate::ui::text_panes::highlight_texts(&texts, "rust"));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Switching the Log's Branch filter: the first page and what it waits on.
#[test]
#[ignore]
fn bench_filter_switch() {
    let root = PathBuf::from(std::env::var("JUNCTION_BENCH").expect("JUNCTION_BENCH"));
    let repository = Repository::discover(&root, GitConsole::default()).unwrap();
    for branches in [vec!["master".to_owned()], vec![], vec!["HEAD".to_owned()]] {
        let filter = git::log::LogFilter { branches, ..Default::default() };
        println!("{:?}", filter.branches);
        time("refs tips", || repository.run(["for-each-ref", "--format=%(objectname) %(refname)"]).unwrap());
        let page = time("first page", || git::log::load_first_page(&repository, &filter).unwrap());
        time("graph", || git::graph::GraphLayout::build(&page));
        time("rows", || git::log::CommitRows::build(&page));
    }
}
