use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use http::header::ACCEPT;
use octocrab::auth::OAuth;
use oo7::Keyring;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};

static GITHUB_CLIENT_ID: LazyLock<SecretString> =
    LazyLock::new(|| SecretString::from(include_str!("../client_id.txt").trim()));
const GITHUB_OAUTH_SCOPE: &[&str] = &[""];
const KEYRING_ATTRIBUTES: &[(&str, &str)] = &[("tool", env!("CARGO_BIN_NAME"))];

pub(crate) async fn get_oauth() -> Result<OAuth> {
    let keyring = oo7::Keyring::new().await.context("create keyring")?;

    // Find a stored secret
    if let Some(oauth) = get_oauth_from_keyring(&keyring)
        .await
        .context("get oauth secret from keyring")?
    {
        return Ok(oauth);
    }

    // no secret found, perform OAuth
    let oauth = perform_oauth().await.context("perform OAuth")?;
    let oauth = OAuthWrapper::from(oauth);

    // Store a secret
    keyring
        .create_item(
            "OAuth secret",
            &KEYRING_ATTRIBUTES,
            serde_json::to_vec(&oauth).context("serialize OAuth")?,
            true,
        )
        .await?;

    Ok(oauth.into())
}

async fn get_oauth_from_keyring(keyring: &Keyring) -> Result<Option<OAuth>> {
    // Find a stored secret
    let items = keyring.search_items(&KEYRING_ATTRIBUTES).await?;
    match items.as_slice() {
        [] => Ok(None),
        [item] => {
            // secret found, load it
            let secret = item.secret().await.context("retrieve secret")?;
            let s = str::from_utf8(secret.as_bytes()).context("decode secret string")?;

            let Ok(oauth) = serde_json::from_str::<OAuthWrapper>(s) else {
                eprintln!("oauth serialization format changed");
                return Ok(None);
            };

            if oauth.scope != GITHUB_OAUTH_SCOPE {
                eprintln!(
                    "oauth scope changed, expected: {GITHUB_OAUTH_SCOPE:?}, got: {:?}",
                    oauth.scope
                );
                return Ok(None);
            }

            if oauth.expires_in.is_some() {
                eprintln!("oauth token potentially expired");
                return Ok(None);
            }

            Ok(Some(oauth.into()))
        }
        _ => {
            bail!("multiple OAuth secrets found")
        }
    }
}

async fn perform_oauth() -> Result<OAuth> {
    let oc = octocrab::Octocrab::builder()
        .base_uri("https://github.com")?
        .add_header(ACCEPT, "application/json".to_string())
        .build()
        .context("create octocrap instance")?;
    let device_codes = oc
        .authenticate_as_device(&GITHUB_CLIENT_ID, GITHUB_OAUTH_SCOPE)
        .await
        .context("set auth flow")?;
    eprintln!(
        "Go go {} and enter following code: {}",
        device_codes.verification_uri, device_codes.user_code
    );
    let oauth = device_codes
        .poll_until_available(&oc, &GITHUB_CLIENT_ID)
        .await
        .context("complete auth flow")?;
    Ok(oauth)
}

/// Clone of [`OAuth`] to implement full (de)-serialization.
#[derive(Debug, Serialize, Deserialize)]
struct OAuthWrapper {
    #[serde(serialize_with = "serialize_secret_string")]
    access_token: SecretString,
    token_type: String,
    scope: Vec<String>,
    expires_in: Option<usize>,
    #[serde(serialize_with = "serialize_opt_secret_string")]
    refresh_token: Option<SecretString>,
    refresh_token_expires_in: Option<usize>,
}

impl From<OAuth> for OAuthWrapper {
    fn from(oauth: OAuth) -> Self {
        Self {
            access_token: oauth.access_token,
            token_type: oauth.token_type,
            scope: oauth.scope,
            expires_in: oauth.expires_in,
            refresh_token: oauth.refresh_token,
            refresh_token_expires_in: oauth.refresh_token_expires_in,
        }
    }
}

impl From<OAuthWrapper> for OAuth {
    fn from(wrapper: OAuthWrapper) -> Self {
        Self {
            access_token: wrapper.access_token,
            token_type: wrapper.token_type,
            scope: wrapper.scope,
            expires_in: wrapper.expires_in,
            refresh_token: wrapper.refresh_token,
            refresh_token_expires_in: wrapper.refresh_token_expires_in,
        }
    }
}

fn serialize_secret_string<S>(string: &SecretString, ser: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    string.expose_secret().serialize(ser)
}

fn serialize_opt_secret_string<S>(string: &Option<SecretString>, ser: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    string.as_ref().map(|s| s.expose_secret()).serialize(ser)
}
