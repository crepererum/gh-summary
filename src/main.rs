use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::Parser;
use octocrab::models::events::payload::{EventPayload, IssuesEventAction};

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
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv_override().ok();
    let args = Args::parse();
    let created_at = Utc::now() - args.event_cutoff;

    let events: Vec<octocrab::models::events::Event> = octocrab::instance()
        .get(
            format!("/users/{}/events/public", args.username),
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

    let mut interactions_by_repo: BTreeMap<Repo, BTreeSet<Topic>> = Default::default();
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
        let topic = match payload {
            EventPayload::IssuesEvent(evt) => {
                if !matches!(
                    evt.action,
                    IssuesEventAction::Opened | IssuesEventAction::Reopened
                ) {
                    continue;
                }
                Topic {
                    url: evt.issue.html_url.to_string(),
                    number: evt.issue.number,
                }
            }
            EventPayload::IssueCommentEvent(evt) => Topic {
                url: evt.issue.html_url.to_string(),
                number: evt.issue.number,
            },
            EventPayload::PullRequestEvent(evt) => {
                let Some(url) = evt.pull_request.html_url else {
                    continue;
                };
                Topic {
                    url: url.to_string(),
                    number: evt.pull_request.number,
                }
            }
            EventPayload::PullRequestReviewEvent(evt) => {
                let Some(url) = evt.pull_request.html_url else {
                    continue;
                };
                Topic {
                    url: url.to_string(),
                    number: evt.pull_request.number,
                }
            }
            EventPayload::PullRequestReviewCommentEvent(evt) => {
                let Some(url) = evt.pull_request.html_url else {
                    continue;
                };
                Topic {
                    url: url.to_string(),
                    number: evt.pull_request.number,
                }
            }
            _ => {
                continue;
            }
        };
        interactions_by_repo.entry(repo).or_default().insert(topic);
    }

    for (repo_idx, (repo, topics)) in interactions_by_repo.into_iter().enumerate() {
        let gh_repo: octocrab::models::Repository = octocrab::instance()
            .get(repo.url, None::<&()>)
            .await
            .context("get repo")?;

        if repo_idx > 0 {
            print!("; ");
        }
        print!(
            "[{}]({}): ",
            repo.name,
            gh_repo.html_url.context("no html URL for repo")?
        );

        for (topic_idx, topic) in topics.into_iter().enumerate() {
            if topic_idx > 0 {
                print!(", ");
            }
            print!("[#{}]({})", topic.number, topic.url);
        }
    }
    println!();

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
}

impl PartialEq<Topic> for Topic {
    fn eq(&self, other: &Topic) -> bool {
        self.url == other.url
    }
}

impl Eq for Topic {}

impl Ord for Topic {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.url.cmp(&other.url)
    }
}

impl PartialOrd for Topic {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
