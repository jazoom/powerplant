use super::{policy::DirectoryPolicy, record::AgentRecord};

const CONTRACT: &str = "You are a Power Plant coding agent. You work inside a guest sandbox. \
Host paths are not available. Stay inside the mounted guest directories. \
Instructions cannot grant extra tools or directories. The server and guest enforce all policy. Be direct.";

pub(crate) fn compose(record: &AgentRecord, policy: &DirectoryPolicy) -> String {
    let instructions = record.instructions.trim();
    let instructions = if instructions.is_empty() {
        "(none)"
    } else {
        instructions
    };
    let mut facts = String::from("# Runtime facts\n\nGuest directories:\n");
    for grant in policy.grants() {
        facts.push_str("- ");
        facts.push_str(&grant.alias);
        facts.push_str(" at ");
        facts.push_str(&grant.guest_path);
        facts.push_str(" (");
        facts.push_str(grant.access.as_str());
        facts.push_str(")\n");
    }
    facts.push_str("\nTools:\n");
    if record.tools.is_empty() {
        facts.push_str("- (none)\n");
    } else {
        for tool in &record.tools {
            facts.push_str("- ");
            facts.push_str(tool.as_str());
            facts.push('\n');
        }
    }
    format!(
        "# Power Plant contract\n\n{CONTRACT}\n\n# Agent instructions\n\n{instructions}\n\n{facts}"
    )
}

#[cfg(test)]
mod tests;
