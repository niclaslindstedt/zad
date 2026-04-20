//! `zad github <verb>` — runtime commands against a configured GitHub
//! PAT.
//!
//! Every verb shells out to the `gh` CLI via a [`GithubTransport`] so
//! `--dry-run` can swap in a preview impl that never spawns the
//! subprocess. Credential resolution mirrors the generic service
//! lifecycle: the project-local config wins over the global one, and
//! the matching keychain entry holds the Personal Access Token. The
//! project must already have enabled the GitHub service.

use std::collections::BTreeSet;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::{self, GithubServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::default_dry_run_sink;
use crate::service::github::permissions::{self as perms, GithubFunction};
use crate::service::github::{DryRunGithubTransport, GhCli, GithubTransport};

// ---------------------------------------------------------------------------
// subcommand tree
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct GithubArgs {
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Issues: list, view, create, comment, close.
    Issue(IssueArgs),
    /// Pull requests: list, view, diff, create, comment, review, merge, checks.
    Pr(PrArgs),
    /// Repositories: view.
    Repo(RepoArgs),
    /// Files: view a file at a ref.
    File(FileArgs),
    /// Code search.
    Code(CodeArgs),
    /// Workflow runs: list, view.
    Run(RunArgs),
    /// Inspect, scaffold, or dry-run the permissions policy.
    Permissions(PermissionsArgs),
}

pub async fn run(args: GithubArgs) -> Result<()> {
    let action = args
        .action
        .ok_or_else(|| ZadError::Invalid("missing subcommand. Run `zad github --help`.".into()))?;
    match action {
        Action::Issue(a) => run_issue(a).await,
        Action::Pr(a) => run_pr(a).await,
        Action::Repo(a) => run_repo(a).await,
        Action::File(a) => run_file(a).await,
        Action::Code(a) => run_code(a).await,
        Action::Run(a) => run_run(a).await,
        Action::Permissions(a) => run_permissions(a),
    }
}

// ---------------------------------------------------------------------------
// credential / config plumbing
// ---------------------------------------------------------------------------

enum EffectiveScope {
    Global,
    Local(String),
}

fn require_github_enabled() -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("github") {
        return Err(ZadError::Invalid(format!(
            "github is not enabled for this project ({}). \
             Run `zad service enable github` first.",
            project_path.display()
        )));
    }
    Ok(())
}

fn effective_config() -> Result<(GithubServiceCfg, EffectiveScope)> {
    require_github_enabled()?;

    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "github")?;
    if let Some(cfg) = config::load_flat::<GithubServiceCfg>(&local_path)? {
        return Ok((cfg, EffectiveScope::Local(slug)));
    }
    let global_path = config::path::global_service_config_path("github")?;
    if let Some(cfg) = config::load_flat::<GithubServiceCfg>(&global_path)? {
        return Ok((cfg, EffectiveScope::Global));
    }
    Err(ZadError::Invalid(format!(
        "no GitHub credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_pat(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::account("github", "pat", Scope::Global),
        EffectiveScope::Local(slug) => secrets::account("github", "pat", Scope::Project(slug)),
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "PAT missing from keychain (account `{account}`). \
             Re-run `zad service create github` to reinstall it."
        ))
    })
}

/// Build a transport for `required` scope. `dry_run` short-circuits
/// the keychain load and returns a preview transport — the scope check
/// still fires so dry-run respects the operator's policy.
fn transport_for(required: &'static str, dry_run: bool) -> Result<Box<dyn GithubTransport>> {
    let (cfg, scope) = effective_config()?;
    let config_path = match &scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "github")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("github")?,
    };
    let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    if !scopes.contains(required) {
        return Err(ZadError::ScopeDenied {
            service: "github",
            scope: required,
            config_path,
        });
    }
    if dry_run {
        return Ok(Box::new(DryRunGithubTransport::new(default_dry_run_sink())));
    }
    let pat = load_pat(&scope)?;
    Ok(Box::new(GhCli::new(&pat, scopes, config_path)))
}

fn resolve_repo(flag: Option<&str>) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    let (cfg, _scope) = effective_config()?;
    cfg.default_repo.ok_or_else(|| {
        ZadError::Invalid(
            "no repo specified: pass --repo owner/name or set `default_repo` in the config".into(),
        )
    })
}

fn resolve_org(flag: Option<&str>) -> Result<Option<String>> {
    if flag.is_some() {
        return Ok(flag.map(str::to_string));
    }
    let (cfg, _scope) = effective_config()?;
    Ok(cfg.default_owner)
}

fn print_passthrough(json: bool, raw: &str, command: &str) {
    if json {
        // `gh --json` already emitted JSON; pass through so piped
        // consumers see the exact shape gh returns.
        print!("{raw}");
    } else if raw.is_empty() {
        println!("{command}: ok");
    } else {
        print!("{raw}");
    }
}

// ---------------------------------------------------------------------------
// issue
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct IssueArgs {
    #[command(subcommand)]
    pub action: IssueAction,
}

#[derive(Debug, Subcommand)]
pub enum IssueAction {
    /// List issues in a repository.
    List(IssueListArgs),
    /// View one issue's body and comments.
    View(IssueViewArgs),
    /// Create a new issue.
    Create(IssueCreateArgs),
    /// Post a comment on an issue.
    Comment(IssueCommentArgs),
    /// Close an issue (optionally with a final comment).
    Close(IssueCloseArgs),
}

async fn run_issue(args: IssueArgs) -> Result<()> {
    match args.action {
        IssueAction::List(a) => run_issue_list(a).await,
        IssueAction::View(a) => run_issue_view(a).await,
        IssueAction::Create(a) => run_issue_create(a).await,
        IssueAction::Comment(a) => run_issue_comment(a).await,
        IssueAction::Close(a) => run_issue_close(a).await,
    }
}

#[derive(Debug, Args)]
pub struct IssueListArgs {
    #[arg(long)]
    pub repo: Option<String>,
    /// open, closed, or all. Default: open.
    #[arg(long, default_value = "open")]
    pub state: String,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

async fn run_issue_list(a: IssueListArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::IssueList)?;
    perms.check_repo(GithubFunction::IssueList, &repo)?;

    let limit = a.limit.to_string();
    let mut gh_args: Vec<&str> = vec![
        "issue",
        "list",
        "--repo",
        &repo,
        "--state",
        &a.state,
        "--limit",
        &limit,
        "--json",
        "number,title,state,author,labels,createdAt,url",
    ];
    if let Some(v) = a.author.as_deref() {
        gh_args.extend_from_slice(&["--author", v]);
    }
    if let Some(v) = a.label.as_deref() {
        gh_args.extend_from_slice(&["--label", v]);
    }

    let t = transport_for("issues.read", false)?;
    let out = t.run("issue_list", &gh_args).await?;
    print_passthrough(a.json, &out, "github.issue.list");
    Ok(())
}

#[derive(Debug, Args)]
pub struct IssueViewArgs {
    /// Issue number.
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub json: bool,
}

async fn run_issue_view(a: IssueViewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::IssueView)?;
    perms.check_repo(GithubFunction::IssueView, &repo)?;

    let number = a.number.to_string();
    let gh_args = [
        "issue",
        "view",
        &number,
        "--repo",
        &repo,
        "--json",
        "number,title,body,state,author,labels,comments,createdAt,updatedAt,url",
    ];
    let t = transport_for("issues.read", false)?;
    let out = t.run("issue_view", &gh_args).await?;
    print_passthrough(a.json, &out, "github.issue.view");
    Ok(())
}

#[derive(Debug, Args)]
pub struct IssueCreateArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub title: String,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long = "label", value_name = "LABEL", action = clap::ArgAction::Append)]
    pub labels: Vec<String>,
    #[arg(long = "assignee", value_name = "USER", action = clap::ArgAction::Append)]
    pub assignees: Vec<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_issue_create(a: IssueCreateArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::IssueCreate)?;
    perms.check_repo(GithubFunction::IssueCreate, &repo)?;
    if let Some(b) = a.body.as_deref() {
        perms.check_body(GithubFunction::IssueCreate, b)?;
    }
    perms.check_body(GithubFunction::IssueCreate, &a.title)?;

    let body = a.body.clone().unwrap_or_default();
    let mut gh_args: Vec<&str> = vec![
        "issue", "create", "--repo", &repo, "--title", &a.title, "--body", &body,
    ];
    for l in &a.labels {
        gh_args.extend_from_slice(&["--label", l]);
    }
    for u in &a.assignees {
        gh_args.extend_from_slice(&["--assignee", u]);
    }

    let t = transport_for("issues.write", a.dry_run)?;
    let summary = format!(
        "would open issue `{}` on {repo}",
        truncate_for_summary(&a.title, 60)
    );
    let details = serde_json::json!({
        "command": "github.issue.create",
        "repo": repo,
        "title": a.title,
        "body": body,
        "labels": a.labels,
        "assignees": a.assignees,
    });
    let out = t
        .run_mutating("issue_create", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.issue.create");
    Ok(())
}

#[derive(Debug, Args)]
pub struct IssueCommentArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub body: String,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_issue_comment(a: IssueCommentArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::IssueComment)?;
    perms.check_repo(GithubFunction::IssueComment, &repo)?;
    perms.check_body(GithubFunction::IssueComment, &a.body)?;

    let number = a.number.to_string();
    let gh_args = [
        "issue", "comment", &number, "--repo", &repo, "--body", &a.body,
    ];
    let t = transport_for("issues.write", a.dry_run)?;
    let summary = format!("would comment on issue #{number} in {repo}");
    let details = serde_json::json!({
        "command": "github.issue.comment",
        "repo": repo,
        "number": number,
        "body": a.body,
    });
    let out = t
        .run_mutating("issue_comment", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.issue.comment");
    Ok(())
}

#[derive(Debug, Args)]
pub struct IssueCloseArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    /// Optional closing comment posted before the state change.
    #[arg(long)]
    pub comment: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_issue_close(a: IssueCloseArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::IssueClose)?;
    perms.check_repo(GithubFunction::IssueClose, &repo)?;
    if let Some(c) = a.comment.as_deref() {
        perms.check_body(GithubFunction::IssueClose, c)?;
    }

    let number = a.number.to_string();
    let mut gh_args: Vec<&str> = vec!["issue", "close", &number, "--repo", &repo];
    if let Some(c) = a.comment.as_deref() {
        gh_args.extend_from_slice(&["--comment", c]);
    }

    let t = transport_for("issues.write", a.dry_run)?;
    let summary = format!("would close issue #{number} in {repo}");
    let details = serde_json::json!({
        "command": "github.issue.close",
        "repo": repo,
        "number": number,
        "comment": a.comment,
    });
    let out = t
        .run_mutating("issue_close", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.issue.close");
    Ok(())
}

// ---------------------------------------------------------------------------
// pr
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PrArgs {
    #[command(subcommand)]
    pub action: PrAction,
}

#[derive(Debug, Subcommand)]
pub enum PrAction {
    /// List pull requests in a repository.
    List(PrListArgs),
    /// View one PR's body, reviews, and conversation.
    View(PrViewArgs),
    /// Print a PR's unified diff.
    Diff(PrDiffArgs),
    /// Open a new pull request.
    Create(PrCreateArgs),
    /// Post a conversation comment on a PR.
    Comment(PrCommentArgs),
    /// File a review (approve, request changes, or comment).
    Review(PrReviewArgs),
    /// Merge a PR (squash, merge, or rebase).
    Merge(PrMergeArgs),
    /// Show CI check summary for a PR.
    Checks(PrChecksArgs),
}

async fn run_pr(args: PrArgs) -> Result<()> {
    match args.action {
        PrAction::List(a) => run_pr_list(a).await,
        PrAction::View(a) => run_pr_view(a).await,
        PrAction::Diff(a) => run_pr_diff(a).await,
        PrAction::Create(a) => run_pr_create(a).await,
        PrAction::Comment(a) => run_pr_comment(a).await,
        PrAction::Review(a) => run_pr_review(a).await,
        PrAction::Merge(a) => run_pr_merge(a).await,
        PrAction::Checks(a) => run_pr_checks(a).await,
    }
}

#[derive(Debug, Args)]
pub struct PrListArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long, default_value = "open")]
    pub state: String,
    #[arg(long)]
    pub author: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub head: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

async fn run_pr_list(a: PrListArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrList)?;
    perms.check_repo(GithubFunction::PrList, &repo)?;

    let limit = a.limit.to_string();
    let mut gh_args: Vec<&str> = vec![
        "pr",
        "list",
        "--repo",
        &repo,
        "--state",
        &a.state,
        "--limit",
        &limit,
        "--json",
        "number,title,state,author,baseRefName,headRefName,isDraft,createdAt,url",
    ];
    if let Some(v) = a.author.as_deref() {
        gh_args.extend_from_slice(&["--author", v]);
    }
    if let Some(v) = a.base.as_deref() {
        gh_args.extend_from_slice(&["--base", v]);
    }
    if let Some(v) = a.head.as_deref() {
        gh_args.extend_from_slice(&["--head", v]);
    }

    let t = transport_for("pulls.read", false)?;
    let out = t.run("pr_list", &gh_args).await?;
    print_passthrough(a.json, &out, "github.pr.list");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrViewArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub json: bool,
}

async fn run_pr_view(a: PrViewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrView)?;
    perms.check_repo(GithubFunction::PrView, &repo)?;

    let number = a.number.to_string();
    let gh_args = [
        "pr",
        "view",
        &number,
        "--repo",
        &repo,
        "--json",
        "number,title,body,state,author,baseRefName,headRefName,isDraft,reviews,comments,mergeable,createdAt,updatedAt,url",
    ];
    let t = transport_for("pulls.read", false)?;
    let out = t.run("pr_view", &gh_args).await?;
    print_passthrough(a.json, &out, "github.pr.view");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrDiffArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
}

async fn run_pr_diff(a: PrDiffArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrDiff)?;
    perms.check_repo(GithubFunction::PrDiff, &repo)?;

    let number = a.number.to_string();
    let gh_args = ["pr", "diff", &number, "--repo", &repo];
    let t = transport_for("pulls.read", false)?;
    let out = t.run("pr_diff", &gh_args).await?;
    print!("{out}");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrCreateArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub title: String,
    #[arg(long)]
    pub body: String,
    /// Base branch (merge target). Omit to let `gh` default to the
    /// repository's default branch.
    #[arg(long)]
    pub base: Option<String>,
    /// Head branch (source). Omit to let `gh` default to the current
    /// branch.
    #[arg(long)]
    pub head: Option<String>,
    #[arg(long)]
    pub draft: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_pr_create(a: PrCreateArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrCreate)?;
    perms.check_repo(GithubFunction::PrCreate, &repo)?;
    perms.check_body(GithubFunction::PrCreate, &a.title)?;
    perms.check_body(GithubFunction::PrCreate, &a.body)?;

    let mut gh_args: Vec<&str> = vec![
        "pr", "create", "--repo", &repo, "--title", &a.title, "--body", &a.body,
    ];
    if a.draft {
        gh_args.push("--draft");
    }
    if let Some(v) = a.base.as_deref() {
        gh_args.extend_from_slice(&["--base", v]);
    }
    if let Some(v) = a.head.as_deref() {
        gh_args.extend_from_slice(&["--head", v]);
    }

    let t = transport_for("pulls.write", a.dry_run)?;
    let summary = format!(
        "would open PR `{}` on {repo}",
        truncate_for_summary(&a.title, 60)
    );
    let details = serde_json::json!({
        "command": "github.pr.create",
        "repo": repo,
        "title": a.title,
        "body": a.body,
        "base": a.base,
        "head": a.head,
        "draft": a.draft,
    });
    let out = t
        .run_mutating("pr_create", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.pr.create");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrCommentArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub body: String,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_pr_comment(a: PrCommentArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrComment)?;
    perms.check_repo(GithubFunction::PrComment, &repo)?;
    perms.check_body(GithubFunction::PrComment, &a.body)?;

    let number = a.number.to_string();
    let gh_args = ["pr", "comment", &number, "--repo", &repo, "--body", &a.body];
    let t = transport_for("pulls.write", a.dry_run)?;
    let summary = format!("would comment on PR #{number} in {repo}");
    let details = serde_json::json!({
        "command": "github.pr.comment",
        "repo": repo,
        "number": number,
        "body": a.body,
    });
    let out = t
        .run_mutating("pr_comment", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.pr.comment");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrReviewArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long, conflicts_with_all = ["request_changes", "comment"])]
    pub approve: bool,
    #[arg(long = "request-changes", conflicts_with_all = ["approve", "comment"])]
    pub request_changes: bool,
    #[arg(long, conflicts_with_all = ["approve", "request_changes"])]
    pub comment: bool,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_pr_review(a: PrReviewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrReview)?;
    perms.check_repo(GithubFunction::PrReview, &repo)?;
    if let Some(b) = a.body.as_deref() {
        perms.check_body(GithubFunction::PrReview, b)?;
    }

    let (mode_flag, mode_label) = if a.approve {
        ("--approve", "approve")
    } else if a.request_changes {
        ("--request-changes", "request-changes")
    } else if a.comment {
        ("--comment", "comment")
    } else {
        return Err(ZadError::Invalid(
            "pass one of --approve, --request-changes, --comment".into(),
        ));
    };
    if (a.request_changes || a.comment) && a.body.is_none() {
        return Err(ZadError::Invalid(format!(
            "`{mode_label}` reviews require --body"
        )));
    }

    let number = a.number.to_string();
    let body = a.body.clone().unwrap_or_default();
    let mut gh_args: Vec<&str> = vec!["pr", "review", &number, "--repo", &repo, mode_flag];
    if !body.is_empty() {
        gh_args.extend_from_slice(&["--body", &body]);
    }

    let t = transport_for("pulls.write", a.dry_run)?;
    let summary = format!("would {mode_label} PR #{number} in {repo}");
    let details = serde_json::json!({
        "command": "github.pr.review",
        "repo": repo,
        "number": number,
        "mode": mode_label,
        "body": body,
    });
    let out = t
        .run_mutating("pr_review", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.pr.review");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrMergeArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long, conflicts_with_all = ["merge", "rebase"])]
    pub squash: bool,
    #[arg(long, conflicts_with_all = ["squash", "rebase"])]
    pub merge: bool,
    #[arg(long, conflicts_with_all = ["squash", "merge"])]
    pub rebase: bool,
    #[arg(long = "delete-branch")]
    pub delete_branch: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_pr_merge(a: PrMergeArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrMerge)?;
    perms.check_repo(GithubFunction::PrMerge, &repo)?;

    let mode_flag = if a.squash {
        "--squash"
    } else if a.merge {
        "--merge"
    } else if a.rebase {
        "--rebase"
    } else {
        return Err(ZadError::Invalid(
            "pass one of --squash, --merge, --rebase".into(),
        ));
    };

    let number = a.number.to_string();
    let mut gh_args: Vec<&str> = vec!["pr", "merge", &number, "--repo", &repo, mode_flag];
    if a.delete_branch {
        gh_args.push("--delete-branch");
    }

    let t = transport_for("pulls.write", a.dry_run)?;
    let mode_label = mode_flag.trim_start_matches("--");
    let summary = format!("would {mode_label}-merge PR #{number} in {repo}");
    let details = serde_json::json!({
        "command": "github.pr.merge",
        "repo": repo,
        "number": number,
        "mode": mode_label,
        "delete_branch": a.delete_branch,
    });
    let out = t
        .run_mutating("pr_merge", &gh_args, summary, details)
        .await?;
    if a.dry_run {
        return Ok(());
    }
    print_passthrough(a.json, &out, "github.pr.merge");
    Ok(())
}

#[derive(Debug, Args)]
pub struct PrChecksArgs {
    pub number: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub json: bool,
}

async fn run_pr_checks(a: PrChecksArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::PrChecks)?;
    perms.check_repo(GithubFunction::PrChecks, &repo)?;

    let number = a.number.to_string();
    let mut gh_args: Vec<&str> = vec!["pr", "checks", &number, "--repo", &repo];
    if a.json {
        gh_args.extend_from_slice(&[
            "--json",
            "name,state,conclusion,startedAt,completedAt,workflow,link",
        ]);
    }
    let t = transport_for("checks.read", false)?;
    let out = t.run("pr_checks", &gh_args).await?;
    print_passthrough(a.json, &out, "github.pr.checks");
    Ok(())
}

// ---------------------------------------------------------------------------
// repo
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub action: RepoAction,
}

#[derive(Debug, Subcommand)]
pub enum RepoAction {
    /// Show repository metadata.
    View(RepoViewArgs),
}

async fn run_repo(args: RepoArgs) -> Result<()> {
    match args.action {
        RepoAction::View(a) => run_repo_view(a).await,
    }
}

#[derive(Debug, Args)]
pub struct RepoViewArgs {
    /// Repo to view (owner/name). Omit to use `default_repo`.
    pub repo: Option<String>,
    #[arg(long)]
    pub json: bool,
}

async fn run_repo_view(a: RepoViewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::RepoView)?;
    perms.check_repo(GithubFunction::RepoView, &repo)?;

    let gh_args = [
        "repo",
        "view",
        &repo,
        "--json",
        "name,owner,description,defaultBranchRef,visibility,isFork,isArchived,stargazerCount,updatedAt,url",
    ];
    let t = transport_for("repo.read", false)?;
    let out = t.run("repo_view", &gh_args).await?;
    print_passthrough(a.json, &out, "github.repo.view");
    Ok(())
}

// ---------------------------------------------------------------------------
// file
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct FileArgs {
    #[command(subcommand)]
    pub action: FileAction,
}

#[derive(Debug, Subcommand)]
pub enum FileAction {
    /// Print the contents of a file at a given ref.
    View(FileViewArgs),
}

async fn run_file(args: FileArgs) -> Result<()> {
    match args.action {
        FileAction::View(a) => run_file_view(a).await,
    }
}

#[derive(Debug, Args)]
pub struct FileViewArgs {
    #[arg(long)]
    pub repo: Option<String>,
    /// Path to the file, repo-relative.
    #[arg(long)]
    pub path: String,
    /// Branch, tag, or commit SHA. Defaults to the repo's default branch.
    #[arg(long = "ref")]
    pub git_ref: Option<String>,
}

async fn run_file_view(a: FileViewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::FileView)?;
    perms.check_repo(GithubFunction::FileView, &repo)?;

    // `gh api repos/<owner>/<name>/contents/<path>` with the `Accept:
    // application/vnd.github.raw` header returns the raw file body. gh
    // handles base64 decoding under that Accept.
    let endpoint = if let Some(r) = a.git_ref.as_deref() {
        format!("repos/{repo}/contents/{}?ref={r}", a.path)
    } else {
        format!("repos/{repo}/contents/{}", a.path)
    };
    let gh_args = ["api", "-H", "Accept: application/vnd.github.raw", &endpoint];
    let t = transport_for("repo.read", false)?;
    let out = t.run("file_view", &gh_args).await?;
    print!("{out}");
    Ok(())
}

// ---------------------------------------------------------------------------
// code search
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CodeArgs {
    #[command(subcommand)]
    pub action: CodeAction,
}

#[derive(Debug, Subcommand)]
pub enum CodeAction {
    /// Full-text code search via GitHub's search API.
    Search(CodeSearchArgs),
}

async fn run_code(args: CodeArgs) -> Result<()> {
    match args.action {
        CodeAction::Search(a) => run_code_search(a).await,
    }
}

#[derive(Debug, Args)]
pub struct CodeSearchArgs {
    /// Query string. Understands GitHub's search syntax
    /// (`fn main extension:rs`, `"exact phrase"`, …).
    pub query: String,
    /// Limit search to one org. Defaults to `default_owner` if set.
    #[arg(long)]
    pub org: Option<String>,
    /// Limit search to one repo (owner/name).
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

async fn run_code_search(a: CodeSearchArgs) -> Result<()> {
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::CodeSearch)?;
    let org = resolve_org(a.org.as_deref())?;
    if let Some(o) = &org {
        perms.check_org(GithubFunction::CodeSearch, o)?;
    }
    if let Some(r) = a.repo.as_deref() {
        perms.check_repo(GithubFunction::CodeSearch, r)?;
    }

    let limit = a.limit.to_string();
    let mut gh_args: Vec<&str> = vec![
        "search",
        "code",
        &a.query,
        "--limit",
        &limit,
        "--json",
        "path,repository,url,textMatches",
    ];
    if let Some(o) = org.as_deref() {
        gh_args.extend_from_slice(&["--owner", o]);
    }
    if let Some(r) = a.repo.as_deref() {
        gh_args.extend_from_slice(&["--repo", r]);
    }
    if let Some(l) = a.language.as_deref() {
        gh_args.extend_from_slice(&["--language", l]);
    }

    let t = transport_for("search", false)?;
    let out = t.run("code_search", &gh_args).await?;
    print_passthrough(a.json, &out, "github.code.search");
    Ok(())
}

// ---------------------------------------------------------------------------
// run (workflow runs)
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub action: RunAction,
}

#[derive(Debug, Subcommand)]
pub enum RunAction {
    /// List recent workflow runs for a repository.
    List(RunListArgs),
    /// Show details for a single run.
    View(RunViewArgs),
}

async fn run_run(args: RunArgs) -> Result<()> {
    match args.action {
        RunAction::List(a) => run_run_list(a).await,
        RunAction::View(a) => run_run_view(a).await,
    }
}

#[derive(Debug, Args)]
pub struct RunListArgs {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub workflow: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

async fn run_run_list(a: RunListArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::RunList)?;
    perms.check_repo(GithubFunction::RunList, &repo)?;

    let limit = a.limit.to_string();
    let mut gh_args: Vec<&str> = vec![
        "run",
        "list",
        "--repo",
        &repo,
        "--limit",
        &limit,
        "--json",
        "databaseId,name,displayTitle,status,conclusion,workflowName,headBranch,createdAt,url",
    ];
    if let Some(w) = a.workflow.as_deref() {
        gh_args.extend_from_slice(&["--workflow", w]);
    }
    let t = transport_for("checks.read", false)?;
    let out = t.run("run_list", &gh_args).await?;
    print_passthrough(a.json, &out, "github.run.list");
    Ok(())
}

#[derive(Debug, Args)]
pub struct RunViewArgs {
    pub run_id: u64,
    #[arg(long)]
    pub repo: Option<String>,
    /// Include a tail of the run log in the output.
    #[arg(long)]
    pub log: bool,
    #[arg(long)]
    pub json: bool,
}

async fn run_run_view(a: RunViewArgs) -> Result<()> {
    let repo = resolve_repo(a.repo.as_deref())?;
    let perms = perms::load_effective()?;
    perms.check_time(GithubFunction::RunView)?;
    perms.check_repo(GithubFunction::RunView, &repo)?;

    let id = a.run_id.to_string();
    let mut gh_args: Vec<&str> = vec!["run", "view", &id, "--repo", &repo];
    if a.log {
        gh_args.push("--log");
    }
    if a.json && !a.log {
        gh_args.extend_from_slice(&[
            "--json",
            "databaseId,name,displayTitle,status,conclusion,workflowName,headBranch,jobs,url",
        ]);
    }
    let t = transport_for("checks.read", false)?;
    let out = t.run("run_view", &gh_args).await?;
    print_passthrough(a.json, &out, "github.run.view");
    Ok(())
}

// ---------------------------------------------------------------------------
// permissions
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,

    /// When no subcommand is given, behave like `show`.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsAction {
    /// Print the effective policy (global + local) for this project.
    Show(PermissionsShowArgs),
    /// Write a starter `permissions.toml` at the selected scope.
    Init(PermissionsInitArgs),
    /// Print the paths considered for this project, in precedence order.
    Path(PermissionsPathArgs),
    /// Dry-run: would this specific call be admitted?
    Check(PermissionsCheckArgs),
    /// Staged-commit workflow (add/remove/commit/sign/etc).
    #[command(flatten)]
    Staging(crate::cli::permissions::StagingAction),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsPathArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsCheckArgs {
    /// Function to check (e.g. `issue_comment`, `pr_merge`).
    #[arg(long)]
    pub function: String,
    /// Repo to test against the `repos` list (`owner/name`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Org to test against the `orgs` list.
    #[arg(long)]
    pub org: Option<String>,
    /// Body to test against the `content` rules.
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub json: bool,
}

fn run_permissions(args: PermissionsArgs) -> Result<()> {
    match args.action {
        None => run_permissions_show(PermissionsShowArgs { json: args.json }),
        Some(PermissionsAction::Show(a)) => run_permissions_show(a),
        Some(PermissionsAction::Init(a)) => run_permissions_init(a),
        Some(PermissionsAction::Path(a)) => run_permissions_path(a),
        Some(PermissionsAction::Check(a)) => run_permissions_check(a),
        Some(PermissionsAction::Staging(a)) => {
            crate::cli::permissions::run::<perms::PermissionsService>(a)
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsShowOutput {
    command: &'static str,
    global: PermissionsScopeBlock,
    local: PermissionsScopeBlock,
}

#[derive(Debug, Serialize)]
struct PermissionsScopeBlock {
    path: String,
    present: bool,
}

fn run_permissions_show(args: PermissionsShowArgs) -> Result<()> {
    let global_p = perms::global_path()?;
    let local_p = perms::local_path_current()?;
    let global_present = global_p.exists();
    let local_present = local_p.exists();

    // Pre-load to surface compile/signature errors up front.
    let _effective = perms::load_effective()?;

    if args.json {
        let out = PermissionsShowOutput {
            command: "github.permissions.show",
            global: PermissionsScopeBlock {
                path: global_p.display().to_string(),
                present: global_present,
            },
            local: PermissionsScopeBlock {
                path: local_p.display().to_string(),
                present: local_present,
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("# permissions");
    println!(
        "  global : {} ({})",
        global_p.display(),
        if global_present {
            "present"
        } else {
            "not present (no restrictions at this scope)"
        }
    );
    println!(
        "  local  : {} ({})",
        local_p.display(),
        if local_present {
            "present"
        } else {
            "not present (no restrictions at this scope)"
        }
    );
    println!();
    if !global_present && !local_present {
        println!("No permission files found. Every declared scope is currently unrestricted.");
        println!("Run `zad github permissions init` to scaffold a starter policy.");
        return Ok(());
    }
    for p in [&global_p, &local_p] {
        if !p.exists() {
            continue;
        }
        println!("## {}", p.display());
        match std::fs::read_to_string(p) {
            Ok(body) => {
                for line in body.lines() {
                    println!("  {line}");
                }
            }
            Err(e) => println!("  (failed to read: {e})"),
        }
        println!();
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsInitOutput {
    command: &'static str,
    scope: &'static str,
    path: String,
    written: bool,
}

fn run_permissions_init(args: PermissionsInitArgs) -> Result<()> {
    let (path, scope) = if args.local {
        (perms::local_path_current()?, "local")
    } else {
        (perms::global_path()?, "global")
    };
    if path.exists() && !args.force {
        return Err(ZadError::Invalid(format!(
            "permissions file already exists at {}. Pass --force to overwrite.",
            path.display()
        )));
    }
    let template = perms::starter_template();
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &template, &key)?;
    if args.json {
        let out = PermissionsInitOutput {
            command: "github.permissions.init",
            scope,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Wrote starter permissions ({scope}): {}", path.display());
        println!("Signed with key {}.", key.fingerprint());
        println!("Write verbs (issue_create, pr_merge, …) are deny-by-default.");
        println!(
            "Edit the file to allow them per repo, then re-sign with \
             `zad github permissions sign`."
        );
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsPathOutput {
    command: &'static str,
    global: String,
    local: String,
}

fn run_permissions_path(args: PermissionsPathArgs) -> Result<()> {
    let global_p = perms::global_path()?;
    let local_p = perms::local_path_current()?;
    if args.json {
        let out = PermissionsPathOutput {
            command: "github.permissions.path",
            global: global_p.display().to_string(),
            local: local_p.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{}", global_p.display());
        println!("{}", local_p.display());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsCheckOutput {
    command: &'static str,
    function: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_path: Option<String>,
}

fn run_permissions_check(args: PermissionsCheckArgs) -> Result<()> {
    let function = GithubFunction::from_name(&args.function).ok_or_else(|| {
        ZadError::Invalid(format!(
            "unknown function `{}`. Expected one of: {}.",
            args.function,
            perms::ALL_FUNCTIONS.join(", ")
        ))
    })?;
    let permissions = perms::load_effective()?;

    let mut outcome: Result<()> = Ok(());
    outcome = outcome.and_then(|()| permissions.check_time(function));
    if outcome.is_ok()
        && let Some(r) = &args.repo
    {
        outcome = permissions.check_repo(function, r);
    }
    if outcome.is_ok()
        && let Some(o) = &args.org
    {
        outcome = permissions.check_org(function, o);
    }
    if outcome.is_ok()
        && let Some(b) = &args.body
    {
        outcome = permissions.check_body(function, b);
    }

    let (allowed, reason, config_path) = match outcome {
        Ok(()) => (true, None, None),
        Err(ZadError::PermissionDenied {
            reason,
            config_path,
            ..
        }) => (false, Some(reason), Some(config_path.display().to_string())),
        Err(e) => return Err(e),
    };

    if args.json {
        let out = PermissionsCheckOutput {
            command: "github.permissions.check",
            function: args.function.clone(),
            allowed,
            reason,
            config_path,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if allowed {
        println!("allow");
    } else {
        println!(
            "deny — {}",
            reason.as_deref().unwrap_or("unspecified reason")
        );
        if let Some(p) = &config_path {
            println!("  config: {p}");
        }
    }
    if !allowed {
        std::process::exit(1);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn truncate_for_summary(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
