use crate::providers::{AuthMethod, ProviderConnection, ProviderKind, SecretString};

pub(super) const SECRET_ENV: &str = "POWERPLANT_API_KEY";

#[derive(Clone, Debug, Default)]
pub(crate) struct GuestAccess {
    pub(crate) host: String,
    pub(crate) secret: Option<SecretString>,
}

impl GuestAccess {
    pub(crate) fn from_connection(connection: &ProviderConnection) -> Self {
        let secret = match connection.auth {
            AuthMethod::ApiKey if !connection.api_key.expose().is_empty() => {
                Some(connection.api_key.clone())
            }
            AuthMethod::ApiKey | AuthMethod::Plan => None,
        };
        Self {
            host: connection.kind.guest_host(connection.auth).to_owned(),
            secret,
        }
    }
}

impl ProviderKind {
    pub(crate) fn guest_host(self, auth: AuthMethod) -> &'static str {
        match (self, auth) {
            (Self::Xai, AuthMethod::Plan) => "cli-chat-proxy.grok.com",
            (Self::Xai, AuthMethod::ApiKey) => "api.x.ai",
            (Self::OpenaiCodex, _) => "api.openai.com",
            (Self::Synthetic, _) => "api.synthetic.new",
            (Self::Openrouter, _) => "openrouter.ai",
            (Self::Deepseek, _) => "api.deepseek.com",
        }
    }
}

pub(super) fn provider_policy(host: &str) -> microsandbox::NetworkPolicy {
    let host = host.trim();
    if host.is_empty() {
        return microsandbox::NetworkPolicy::none();
    }
    let mut policy = match microsandbox::NetworkPolicy::builder()
        .default_deny()
        .egress(|rule| rule.allow_domains([host]))
        .build()
    {
        Ok(policy) => policy,
        Err(_) => return microsandbox::NetworkPolicy::none(),
    };
    // Domain allows do not open the resolver. DNS must stay available for that host.
    policy
        .rules
        .insert(0, microsandbox::NetworkRule::allow_dns());
    policy
}

#[cfg(test)]
mod tests;
