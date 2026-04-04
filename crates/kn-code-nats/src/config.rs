use secrecy::Secret;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    pub url: String,
    #[serde(skip_serializing)]
    pub auth: NatsAuth,
    pub instance_id: Option<String>,
    pub startup_subs: Vec<String>,
    pub request_timeout_ms: u64,
    pub heartbeat_interval_secs: u64,
    pub agent_ttl_secs: u64,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub enum NatsAuth {
    #[default]
    None,
    Token(Secret<String>),
    UserPass {
        user: String,
        pass: Secret<String>,
    },
    NKey(Secret<String>),
}

impl std::fmt::Debug for NatsAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NatsAuth::None => write!(f, "None"),
            NatsAuth::Token(_) => write!(f, "Token(***)"),
            NatsAuth::UserPass { user, .. } => {
                write!(f, "UserPass {{ user: {:?}, pass: *** }}", user)
            }
            NatsAuth::NKey(_) => write!(f, "NKey(***)"),
        }
    }
}

impl serde::Serialize for NatsAuth {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            NatsAuth::None => serializer.serialize_str("none"),
            NatsAuth::Token(_) => serializer.serialize_str("token(***)"),
            NatsAuth::UserPass { user, .. } => {
                use serde::ser::SerializeStruct;
                let mut s = serializer.serialize_struct("UserPass", 2)?;
                s.serialize_field("user", user)?;
                s.serialize_field("pass", "***")?;
                s.end()
            }
            NatsAuth::NKey(_) => serializer.serialize_str("nkey(***)"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for NatsAuth {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AuthHelper {
            #[serde(default)]
            r#type: String,
            #[serde(default)]
            user: String,
            #[serde(default)]
            pass: String,
            #[serde(default)]
            token: String,
            #[serde(default)]
            nkey: String,
        }

        let helper = AuthHelper::deserialize(deserializer)?;
        match helper.r#type.as_str() {
            "token" => Ok(NatsAuth::Token(Secret::new(helper.token))),
            "userpass" => Ok(NatsAuth::UserPass {
                user: helper.user,
                pass: Secret::new(helper.pass),
            }),
            "nkey" => Ok(NatsAuth::NKey(Secret::new(helper.nkey))),
            _ => Ok(NatsAuth::None),
        }
    }
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("KN_CODE_NATS_URL")
                .or_else(|_| std::env::var("NUNTIUS_NATS_URL"))
                .unwrap_or_else(|_| "tls://localhost:4222".to_string()),
            auth: NatsAuth::default(),
            instance_id: std::env::var("KN_CODE_INSTANCE_ID")
                .or_else(|_| std::env::var("NUNTIUS_INSTANCE_ID"))
                .ok(),
            startup_subs: std::env::var("KN_CODE_NATS_STARTUP_SUBS")
                .or_else(|_| std::env::var("NUNTIUS_STARTUP_SUBS"))
                .ok()
                .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            request_timeout_ms: 5000,
            heartbeat_interval_secs: 120,
            agent_ttl_secs: 300,
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl NatsConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(token) =
            std::env::var("KN_CODE_NATS_TOKEN").or_else(|_| std::env::var("NUNTIUS_AUTH_TOKEN"))
        {
            config.auth = NatsAuth::Token(Secret::new(token));
        } else if let (Ok(user), Ok(pass)) = (
            std::env::var("KN_CODE_NATS_USER").or_else(|_| std::env::var("NUNTIUS_USER")),
            std::env::var("KN_CODE_NATS_PASS").or_else(|_| std::env::var("NUNTIUS_PASS")),
        ) {
            config.auth = NatsAuth::UserPass {
                user,
                pass: Secret::new(pass),
            };
        } else if let Ok(nkey) =
            std::env::var("KN_CODE_NATS_NKEY").or_else(|_| std::env::var("NUNTIUS_NKEY"))
        {
            config.auth = NatsAuth::NKey(Secret::new(nkey));
        }

        config
    }

    pub fn instance_id(&self) -> String {
        self.instance_id.clone().unwrap_or_else(|| {
            let uuid = uuid::Uuid::new_v4().to_string();
            uuid[..8].to_string()
        })
    }
}
