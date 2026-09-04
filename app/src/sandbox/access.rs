use crate::agents::NetworkAccess;

pub(crate) fn public_network_policy() -> microsandbox::NetworkPolicy {
    let mut policy =
        microsandbox::NetworkPolicy::from_profiles([microsandbox::NetworkProfile::Public]);
    policy.default_ingress = microsandbox::NetworkAction::Deny;
    policy
}

pub(super) fn network_policy(access: &NetworkAccess) -> microsandbox::NetworkPolicy {
    match access {
        NetworkAccess::None => microsandbox::NetworkPolicy::none(),
        NetworkAccess::Public => public_network_policy(),
        NetworkAccess::Restricted(domains) => restricted_network_policy(domains),
    }
}

fn restricted_network_policy(domains: &[String]) -> microsandbox::NetworkPolicy {
    if domains.is_empty() {
        return microsandbox::NetworkPolicy::none();
    }
    let mut policy = match microsandbox::NetworkPolicy::builder()
        .default_deny()
        .egress(|rules| rules.allow_domain_suffixes(domains.iter().map(String::as_str)))
        .build()
    {
        Ok(policy) => policy,
        Err(_) => return microsandbox::NetworkPolicy::none(),
    };
    policy
        .rules
        .insert(0, microsandbox::NetworkRule::allow_dns());
    policy
}

#[cfg(test)]
mod tests;
