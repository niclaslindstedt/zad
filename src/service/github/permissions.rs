//! GitHub-specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/github/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/github/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! apply — a call must pass **both**. Local can only tighten global,
//! never loosen it.
//!
//! Permission targets are `repos` (matched as `owner/name`) and
//! `orgs`. Patterns reuse the shared [`PatternList`] grammar: exact
//! match, glob (`*`/`?`), or `re:<regex>`. A pattern like `myorg/*`
//! allow-lists every repo in that org; `*/docs` allow-lists any repo
//! named `docs` under any owner.
//!
//! Every runtime verb has its own per-function block; read verbs
//! default to `*` (broad allow), write verbs ship deny-by-default in
//! the starter template and must be explicitly opted in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    mutation::{self, Mutation},
    pattern::{PatternList, PatternListRaw},
    service::HasSignature,
    signing::{self, Signature},
    time::{TimeWindow, TimeWindowRaw},
};

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

/// Raw on-disk policy. Every field is optional; the `Default` impl
/// corresponds to "no restrictions at this layer".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubPermissionsRaw {
    #[serde(default)]
    pub content: ContentRulesRaw,
    #[serde(default)]
    pub time: TimeWindowRaw,

    #[serde(default)]
    pub issue_list: FunctionBlockRaw,
    #[serde(default)]
    pub issue_view: FunctionBlockRaw,
    #[serde(default)]
    pub issue_create: FunctionBlockRaw,
    #[serde(default)]
    pub issue_comment: FunctionBlockRaw,
    #[serde(default)]
    pub issue_close: FunctionBlockRaw,

    #[serde(default)]
    pub pr_list: FunctionBlockRaw,
    #[serde(default)]
    pub pr_view: FunctionBlockRaw,
    #[serde(default)]
    pub pr_diff: FunctionBlockRaw,
    #[serde(default)]
    pub pr_create: FunctionBlockRaw,
    #[serde(default)]
    pub pr_comment: FunctionBlockRaw,
    #[serde(default)]
    pub pr_review: FunctionBlockRaw,
    #[serde(default)]
    pub pr_merge: FunctionBlockRaw,
    #[serde(default)]
    pub pr_checks: FunctionBlockRaw,

    #[serde(default)]
    pub repo_view: FunctionBlockRaw,
    #[serde(default)]
    pub file_view: FunctionBlockRaw,
    #[serde(default)]
    pub code_search: FunctionBlockRaw,
    #[serde(default)]
    pub run_list: FunctionBlockRaw,
    #[serde(default)]
    pub run_view: FunctionBlockRaw,

    /// Ed25519 signature over the canonical serialization of every
    /// other field. Enforced at load time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl HasSignature for GithubPermissionsRaw {
    fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }
    fn set_signature(&mut self, sig: Option<Signature>) {
        self.signature = sig;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "pattern_list_is_default")]
    pub repos: PatternListRaw,
    #[serde(default, skip_serializing_if = "pattern_list_is_default")]
    pub orgs: PatternListRaw,
    #[serde(default, skip_serializing_if = "content_rules_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "time_window_is_default")]
    pub time: TimeWindowRaw,
}

fn pattern_list_is_default(v: &PatternListRaw) -> bool {
    v.allow.is_empty() && v.deny.is_empty()
}
fn content_rules_is_default(v: &ContentRulesRaw) -> bool {
    v.deny_words.is_empty() && v.deny_patterns.is_empty() && v.max_length.is_none()
}
fn time_window_is_default(v: &TimeWindowRaw) -> bool {
    v.days.is_empty() && v.windows.is_empty()
}

// ---------------------------------------------------------------------------
// compiled form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FunctionBlock {
    pub repos: PatternList,
    pub orgs: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            repos: PatternList::compile(&raw.repos).map_err(ZadError::Invalid)?,
            orgs: PatternList::compile(&raw.orgs).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
        })
    }
}

/// One file's worth of rules, compiled.
#[derive(Debug, Clone, Default)]
pub struct GithubPermissions {
    /// Path the rules were loaded from — embedded in every
    /// `PermissionDenied` error so operators know where to edit.
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,

    pub issue_list: FunctionBlock,
    pub issue_view: FunctionBlock,
    pub issue_create: FunctionBlock,
    pub issue_comment: FunctionBlock,
    pub issue_close: FunctionBlock,

    pub pr_list: FunctionBlock,
    pub pr_view: FunctionBlock,
    pub pr_diff: FunctionBlock,
    pub pr_create: FunctionBlock,
    pub pr_comment: FunctionBlock,
    pub pr_review: FunctionBlock,
    pub pr_merge: FunctionBlock,
    pub pr_checks: FunctionBlock,

    pub repo_view: FunctionBlock,
    pub file_view: FunctionBlock,
    pub code_search: FunctionBlock,
    pub run_list: FunctionBlock,
    pub run_view: FunctionBlock,
}

impl GithubPermissions {
    fn compile(raw: &GithubPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(GithubPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            issue_list: FunctionBlock::compile(&raw.issue_list)?,
            issue_view: FunctionBlock::compile(&raw.issue_view)?,
            issue_create: FunctionBlock::compile(&raw.issue_create)?,
            issue_comment: FunctionBlock::compile(&raw.issue_comment)?,
            issue_close: FunctionBlock::compile(&raw.issue_close)?,
            pr_list: FunctionBlock::compile(&raw.pr_list)?,
            pr_view: FunctionBlock::compile(&raw.pr_view)?,
            pr_diff: FunctionBlock::compile(&raw.pr_diff)?,
            pr_create: FunctionBlock::compile(&raw.pr_create)?,
            pr_comment: FunctionBlock::compile(&raw.pr_comment)?,
            pr_review: FunctionBlock::compile(&raw.pr_review)?,
            pr_merge: FunctionBlock::compile(&raw.pr_merge)?,
            pr_checks: FunctionBlock::compile(&raw.pr_checks)?,
            repo_view: FunctionBlock::compile(&raw.repo_view)?,
            file_view: FunctionBlock::compile(&raw.file_view)?,
            code_search: FunctionBlock::compile(&raw.code_search)?,
            run_list: FunctionBlock::compile(&raw.run_list)?,
            run_view: FunctionBlock::compile(&raw.run_view)?,
        })
    }

    fn block(&self, f: GithubFunction) -> &FunctionBlock {
        match f {
            GithubFunction::IssueList => &self.issue_list,
            GithubFunction::IssueView => &self.issue_view,
            GithubFunction::IssueCreate => &self.issue_create,
            GithubFunction::IssueComment => &self.issue_comment,
            GithubFunction::IssueClose => &self.issue_close,
            GithubFunction::PrList => &self.pr_list,
            GithubFunction::PrView => &self.pr_view,
            GithubFunction::PrDiff => &self.pr_diff,
            GithubFunction::PrCreate => &self.pr_create,
            GithubFunction::PrComment => &self.pr_comment,
            GithubFunction::PrReview => &self.pr_review,
            GithubFunction::PrMerge => &self.pr_merge,
            GithubFunction::PrChecks => &self.pr_checks,
            GithubFunction::RepoView => &self.repo_view,
            GithubFunction::FileView => &self.file_view,
            GithubFunction::CodeSearch => &self.code_search,
            GithubFunction::RunList => &self.run_list,
            GithubFunction::RunView => &self.run_view,
        }
    }
}

/// Every GitHub runtime function with a permissions gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubFunction {
    IssueList,
    IssueView,
    IssueCreate,
    IssueComment,
    IssueClose,
    PrList,
    PrView,
    PrDiff,
    PrCreate,
    PrComment,
    PrReview,
    PrMerge,
    PrChecks,
    RepoView,
    FileView,
    CodeSearch,
    RunList,
    RunView,
}

impl GithubFunction {
    pub fn name(self) -> &'static str {
        match self {
            GithubFunction::IssueList => "issue_list",
            GithubFunction::IssueView => "issue_view",
            GithubFunction::IssueCreate => "issue_create",
            GithubFunction::IssueComment => "issue_comment",
            GithubFunction::IssueClose => "issue_close",
            GithubFunction::PrList => "pr_list",
            GithubFunction::PrView => "pr_view",
            GithubFunction::PrDiff => "pr_diff",
            GithubFunction::PrCreate => "pr_create",
            GithubFunction::PrComment => "pr_comment",
            GithubFunction::PrReview => "pr_review",
            GithubFunction::PrMerge => "pr_merge",
            GithubFunction::PrChecks => "pr_checks",
            GithubFunction::RepoView => "repo_view",
            GithubFunction::FileView => "file_view",
            GithubFunction::CodeSearch => "code_search",
            GithubFunction::RunList => "run_list",
            GithubFunction::RunView => "run_view",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "issue_list" => GithubFunction::IssueList,
            "issue_view" => GithubFunction::IssueView,
            "issue_create" => GithubFunction::IssueCreate,
            "issue_comment" => GithubFunction::IssueComment,
            "issue_close" => GithubFunction::IssueClose,
            "pr_list" => GithubFunction::PrList,
            "pr_view" => GithubFunction::PrView,
            "pr_diff" => GithubFunction::PrDiff,
            "pr_create" => GithubFunction::PrCreate,
            "pr_comment" => GithubFunction::PrComment,
            "pr_review" => GithubFunction::PrReview,
            "pr_merge" => GithubFunction::PrMerge,
            "pr_checks" => GithubFunction::PrChecks,
            "repo_view" => GithubFunction::RepoView,
            "file_view" => GithubFunction::FileView,
            "code_search" => GithubFunction::CodeSearch,
            "run_list" => GithubFunction::RunList,
            "run_view" => GithubFunction::RunView,
            _ => return None,
        })
    }
}

pub const ALL_FUNCTIONS: &[&str] = &[
    "issue_list",
    "issue_view",
    "issue_create",
    "issue_comment",
    "issue_close",
    "pr_list",
    "pr_view",
    "pr_diff",
    "pr_create",
    "pr_comment",
    "pr_review",
    "pr_merge",
    "pr_checks",
    "repo_view",
    "file_view",
    "code_search",
    "run_list",
    "run_view",
];

pub const TARGET_KINDS: &[&str] = &["repo", "org"];

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<GithubPermissions>,
    pub local: Option<GithubPermissions>,
}

impl EffectivePermissions {
    pub fn any(&self) -> bool {
        self.global.is_some() || self.local.is_some()
    }

    pub fn sources(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = vec![];
        if let Some(g) = &self.global {
            out.push(&g.source);
        }
        if let Some(l) = &self.local {
            out.push(&l.source);
        }
        out
    }

    fn layers(&self) -> impl Iterator<Item = &GithubPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    /// Time-window check for a given function. Called at the top of
    /// every runtime verb, so the denied response never leaks the
    /// target string.
    pub fn check_time(&self, f: GithubFunction) -> Result<()> {
        for p in self.layers() {
            let merged = p.time.clone().merge(p.block(f).time.clone());
            if let Err(e) = merged.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Evaluate a repo target against the `repos` list on the
    /// per-function block. Input is the raw `owner/name` string the
    /// caller passed via `--repo`.
    pub fn check_repo(&self, f: GithubFunction, repo: &str) -> Result<()> {
        for p in self.layers() {
            let list = &p.block(f).repos;
            if list.is_empty() {
                continue;
            }
            if let Err(e) = list.evaluate(std::iter::once(repo)) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("repo `{repo}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Evaluate an org target against the `orgs` list on the
    /// per-function block. Used by `code_search` and any verb that
    /// takes `--org`.
    pub fn check_org(&self, f: GithubFunction, org: &str) -> Result<()> {
        for p in self.layers() {
            let list = &p.block(f).orgs;
            if list.is_empty() {
                continue;
            }
            if let Err(e) = list.evaluate(std::iter::once(org)) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("org `{org}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Content check for the body of a mutating verb (issue/PR create
    /// or comment). Applies the top-level `[content]` defaults merged
    /// with the per-function `content` override.
    pub fn check_body(&self, f: GithubFunction, body: &str) -> Result<()> {
        for p in self.layers() {
            let merged = p.content.clone().merge(p.block(f).content.clone());
            if let Err(e) = merged.evaluate(body) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    Ok(config::path::global_service_dir("github")?.join("permissions.toml"))
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, "github")?.join("permissions.toml"))
}

pub fn local_path_current() -> Result<PathBuf> {
    local_path_for(&config::path::project_slug()?)
}

pub fn load_file(path: &Path) -> Result<Option<GithubPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: GithubPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = GithubPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

fn wrap_compile_error(err: ZadError, path: &Path) -> ZadError {
    match err {
        ZadError::Invalid(msg) => ZadError::Invalid(format!(
            "invalid permissions file {}: {msg}",
            path.display()
        )),
        other => other,
    }
}

pub fn load_effective() -> Result<EffectivePermissions> {
    let slug = config::path::project_slug()?;
    load_effective_for(&slug)
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(
    path: &Path,
    raw: &GithubPermissionsRaw,
    key: &crate::permissions::signing::SigningKey,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let sig = signing::sign_raw(&to_write, key)?;
    to_write.set_signature(Some(sig));
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Starter policy. Read verbs allow `*`; every write verb is
/// deny-by-default so the operator opts in per repo.
pub fn starter_template() -> GithubPermissionsRaw {
    let allow_star = FunctionBlockRaw {
        repos: PatternListRaw {
            allow: vec!["*".into()],
            deny: vec![],
        },
        ..FunctionBlockRaw::default()
    };
    let allow_star_org = FunctionBlockRaw {
        orgs: PatternListRaw {
            allow: vec!["*".into()],
            deny: vec![],
        },
        ..FunctionBlockRaw::default()
    };
    let deny_all = FunctionBlockRaw {
        repos: PatternListRaw {
            allow: vec![],
            deny: vec!["*".into()],
        },
        ..FunctionBlockRaw::default()
    };

    GithubPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret_token".into()],
            deny_patterns: vec![
                // Catch accidentally-pasted PATs and bearer tokens.
                r"(?i)ghp_[A-Za-z0-9]{30,}".into(),
                r"(?i)github_pat_[A-Za-z0-9_]{30,}".into(),
                r"(?i)bearer\s+[A-Za-z0-9\-_.]{20,}".into(),
            ],
            max_length: Some(20000),
        },
        time: TimeWindowRaw::default(),

        // Read verbs — broad allow by default.
        issue_list: allow_star.clone(),
        issue_view: allow_star.clone(),
        pr_list: allow_star.clone(),
        pr_view: allow_star.clone(),
        pr_diff: allow_star.clone(),
        pr_checks: allow_star.clone(),
        repo_view: allow_star.clone(),
        file_view: allow_star.clone(),
        run_list: allow_star.clone(),
        run_view: allow_star.clone(),
        code_search: allow_star_org,

        // Write verbs — deny-by-default; operator opts in per repo.
        issue_create: deny_all.clone(),
        issue_comment: deny_all.clone(),
        issue_close: deny_all.clone(),
        pr_create: deny_all.clone(),
        pr_comment: deny_all.clone(),
        pr_review: deny_all.clone(),
        pr_merge: deny_all,

        signature: None,
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding for the staged-commit CLI
// ---------------------------------------------------------------------------

pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "github";
    type Raw = GithubPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        ALL_FUNCTIONS
    }

    fn target_kinds() -> &'static [&'static str] {
        TARGET_KINDS
    }

    fn apply_mutation(raw: &mut Self::Raw, m: &Mutation) -> Result<()> {
        let function = match m {
            Mutation::AddPattern { function, .. }
            | Mutation::RemovePattern { function, .. }
            | Mutation::AddDenyWord { function, .. }
            | Mutation::RemoveDenyWord { function, .. }
            | Mutation::AddDenyRegex { function, .. }
            | Mutation::RemoveDenyRegex { function, .. }
            | Mutation::SetMaxLength { function, .. }
            | Mutation::SetTimeDays { function, .. }
            | Mutation::SetTimeWindows { function, .. } => function.as_deref(),
        };

        let (content, time) = block_refs_mut(raw, function)?;
        if mutation::apply_content(content, m)? {
            return Ok(());
        }
        if mutation::apply_time(time, m)? {
            return Ok(());
        }

        match m {
            Mutation::AddPattern {
                function,
                target,
                list,
                value,
            }
            | Mutation::RemovePattern {
                function,
                target,
                list,
                value,
            } => {
                let add = matches!(m, Mutation::AddPattern { .. });
                let plist = pattern_list_mut(raw, function.as_deref(), target)?;
                mutation::apply_pattern_list(plist, *list, value, add);
                Ok(())
            }
            other => Err(mutation::unsupported("github", other)),
        }
    }
}

fn function_block_mut<'a>(
    raw: &'a mut GithubPermissionsRaw,
    function: &str,
) -> Result<&'a mut FunctionBlockRaw> {
    Ok(match function {
        "issue_list" => &mut raw.issue_list,
        "issue_view" => &mut raw.issue_view,
        "issue_create" => &mut raw.issue_create,
        "issue_comment" => &mut raw.issue_comment,
        "issue_close" => &mut raw.issue_close,
        "pr_list" => &mut raw.pr_list,
        "pr_view" => &mut raw.pr_view,
        "pr_diff" => &mut raw.pr_diff,
        "pr_create" => &mut raw.pr_create,
        "pr_comment" => &mut raw.pr_comment,
        "pr_review" => &mut raw.pr_review,
        "pr_merge" => &mut raw.pr_merge,
        "pr_checks" => &mut raw.pr_checks,
        "repo_view" => &mut raw.repo_view,
        "file_view" => &mut raw.file_view,
        "code_search" => &mut raw.code_search,
        "run_list" => &mut raw.run_list,
        "run_view" => &mut raw.run_view,
        other => {
            return Err(ZadError::Invalid(format!(
                "github permissions: unknown function `{other}`; expected one of {}",
                ALL_FUNCTIONS.join(", ")
            )));
        }
    })
}

fn block_refs_mut<'a>(
    raw: &'a mut GithubPermissionsRaw,
    function: Option<&str>,
) -> Result<(&'a mut ContentRulesRaw, &'a mut TimeWindowRaw)> {
    match function {
        None => Ok((&mut raw.content, &mut raw.time)),
        Some(name) => {
            let block = function_block_mut(raw, name)?;
            Ok((&mut block.content, &mut block.time))
        }
    }
}

fn pattern_list_mut<'a>(
    raw: &'a mut GithubPermissionsRaw,
    function: Option<&str>,
    target: &str,
) -> Result<&'a mut PatternListRaw> {
    let Some(name) = function else {
        return Err(ZadError::Invalid(
            "github permissions: pattern mutations require --function (there are no \
             top-level repo/org lists in the github schema)"
                .into(),
        ));
    };
    let block = function_block_mut(raw, name)?;
    Ok(match target {
        "repo" => &mut block.repos,
        "org" => &mut block.orgs,
        other => {
            return Err(ZadError::Invalid(format!(
                "github permissions: unknown target `{other}`; expected one of repo, org"
            )));
        }
    })
}
