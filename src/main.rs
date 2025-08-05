use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
    time::Duration,
};

use anyhow::{Context, Error, Result, bail};
use chrono::Utc;
use clap::Parser;
use octocrab::models::events::payload::{EventPayload, IssuesEventAction, PullRequestEventAction};
use regex::Regex;

static UNSAFE_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[^0-9a-zA-Z /():;.&+-]"#).expect("valid regex"));
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\s+"#).expect("valid regex"));

#[derive(Parser)]
struct Args {
    /// Event cutoff by creation date.
    #[clap(long, default_value="1 week", value_parser=humantime::parse_duration)]
    event_cutoff: Duration,

    /// Number of events to fetch.
    #[clap(long, default_value_t = 1000)]
    n_events: u64,

    /// Orgs.
    #[clap(
        long,
        required = false,
        num_args=1..,
        value_delimiter = ',',
    )]
    orgs: Vec<String>,

    /// Username.
    #[clap(long)]
    username: String,

    /// User access token.
    #[clap(long, env = "GITHUB_USER_ACCESS_TOKEN")]
    user_access_token: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv_override().ok();
    let args = Args::parse();
    let created_at = Utc::now() - args.event_cutoff;

    let mut oc_builder = octocrab::Octocrab::builder();
    if let Some(token) = args.user_access_token {
        oc_builder = oc_builder.user_access_token(token);
    }
    let oc = oc_builder.build().context("create octocrap instance")?;

    let events: Vec<octocrab::models::events::Event> = oc
        .get(
            format!("/users/{}/events", args.username),
            Some(&[("per_page", args.n_events)]),
        )
        .await
        .context("list events")?;

    if !events.iter().any(|evt| evt.created_at < created_at) {
        bail!(
            "number of events ({}) to low for give time period ({})",
            args.n_events,
            humantime::format_duration(args.event_cutoff)
        );
    }

    let mut interactions_by_repo: BTreeMap<Repo, BTreeMap<Topic, BTreeSet<Action>>> =
        Default::default();
    for event in events {
        if !event.public {
            continue;
        }

        if !args.orgs.is_empty()
            && !args
                .orgs
                .iter()
                .any(|org| event.repo.name.starts_with(&format!("{org}/")))
        {
            continue;
        }

        if event.created_at < created_at {
            continue;
        }

        let repo = Repo {
            name: event.repo.name,
            url: event.repo.url.to_string(),
        };

        let Some(payload) = event.payload else {
            continue;
        };
        let Some(payload) = payload.specific else {
            continue;
        };
        let (topic, action) = match payload {
            EventPayload::IssuesEvent(evt) => {
                if !matches!(
                    evt.action,
                    IssuesEventAction::Opened | IssuesEventAction::Reopened
                ) {
                    continue;
                }
                (Topic::from(evt.issue), Action::Write)
            }
            EventPayload::IssueCommentEvent(evt) => (Topic::from(evt.issue), Action::Comment),
            EventPayload::PullRequestEvent(evt) => {
                if !matches!(
                    evt.action,
                    PullRequestEventAction::Opened | PullRequestEventAction::Reopened
                ) {
                    continue;
                }
                (
                    Topic::try_from(evt.pull_request).context("convert PR data")?,
                    Action::Code,
                )
            }
            EventPayload::PullRequestReviewEvent(evt) => (
                Topic::try_from(evt.pull_request).context("convert PR data")?,
                Action::Review,
            ),
            EventPayload::PullRequestReviewCommentEvent(evt) => (
                Topic::try_from(evt.pull_request).context("convert PR data")?,
                Action::Review,
            ),
            _ => {
                continue;
            }
        };
        interactions_by_repo
            .entry(repo)
            .or_default()
            .entry(topic)
            .or_default()
            .insert(action);
    }

    for (repo, topics) in interactions_by_repo.into_iter() {
        let gh_repo: octocrab::models::Repository = octocrab::instance()
            .get(&repo.url, None::<&()>)
            .await
            .with_context(|| format!("get repo: {}", repo.url))?;

        print!(
            "- *[{}]({}):*",
            repo.name,
            gh_repo.html_url.context("no html URL for repo")?
        );

        for (topic_idx, (topic, actions)) in topics.into_iter().enumerate() {
            if topic_idx > 0 {
                print!(",");
            }
            // EN space
            print!("\u{2000}");

            for action in actions.into_iter() {
                print!("{action}");
            }
            print!(" {topic}");
        }

        println!();
    }

    Ok(())
}

struct Repo {
    name: String,
    url: String,
}

impl PartialEq<Repo> for Repo {
    fn eq(&self, other: &Repo) -> bool {
        self.name == other.name
    }
}

impl Eq for Repo {}

impl Ord for Repo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for Repo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct Topic {
    url: String,
    number: u64,
    title: String,
}

impl PartialEq<Topic> for Topic {
    fn eq(&self, other: &Topic) -> bool {
        self.number == other.number
    }
}

impl Eq for Topic {}

impl Ord for Topic {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.number.cmp(&other.number)
    }
}

impl PartialOrd for Topic {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { url, number, title } = self;

        let title = UNSAFE_CHARS.replace_all(title, "");
        let title = WHITESPACE.replace_all(&title, " ");

        write!(f, "[#{number}]({url}) (_{title}_)")
    }
}

impl From<octocrab::models::issues::Issue> for Topic {
    fn from(issue: octocrab::models::issues::Issue) -> Self {
        Self {
            url: issue.html_url.to_string(),
            number: issue.number,
            title: issue.title,
        }
    }
}

impl TryFrom<octocrab::models::pulls::PullRequest> for Topic {
    type Error = Error;

    fn try_from(pr: octocrab::models::pulls::PullRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            url: pr.html_url.context("HTML URL missing")?.to_string(),
            number: pr.number,
            title: pr.title.context("PR title missing")?,
        })
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Action {
    Code,
    Write,
    Review,
    Comment,
}

impl Action {
    fn as_str(&self) -> &'static str {
        match self {
            Action::Code => "🔨",
            Action::Write => "✍️",
            Action::Review => "🕵️",
            Action::Comment => "💬",
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
