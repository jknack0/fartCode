//! GitHub client (E4-07/E4-09, #47/#49): token source (keyring + `gh auth
//! token` import), typed REST DTOs, and the `reqwest` client.
//!
//! PRD: no Octokit — plain REST over `reqwest`. Until E8 (accounts) auth is
//! minimal: one token in the OS keyring, never in SQLite/logs.

pub mod client;
pub mod models;
pub mod token;

pub use client::{parse_github_slug, GitHubClient, DEFAULT_API_BASE};
pub use models::{
    PrCheckDto, PrCommentDto, PrCommentKind, PrCommitDto, PrDto, PrFileDto, PrStatus, PrUserDto,
};
