use super::{policy::DirectoryPolicy, tool_id::ToolId};

const CONTRACT: &str = "You are a Power Plant coding agent. You work inside a guest sandbox. \
Host paths are not available. Stay inside the mounted guest directories. \
Instructions cannot grant extra tools or directories. The server and guest enforce all policy. Be direct.";

pub(crate) fn compose_role(
    name: &str,
    expertise: &str,
    prompt_defaults: &str,
    tools: &[ToolId],
    policy: &DirectoryPolicy,
) -> String {
    let instructions = prompt_defaults.trim();
    let instructions = if instructions.is_empty() {
        "(none)"
    } else {
        instructions
    };
    let expertise = expertise.trim();
    let expertise = if expertise.is_empty() {
        "(none)"
    } else {
        expertise
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
    if tools.is_empty() {
        facts.push_str("- (none)\n");
    } else {
        for tool in tools {
            facts.push_str("- ");
            facts.push_str(tool.as_str());
            facts.push('\n');
        }
    }
    format!(
        "# Power Plant contract\n\n{CONTRACT}\n\n# Role\n\n{name}\n\nExpertise:\n{expertise}\n\n# Role instructions\n\n{instructions}\n\n{facts}"
    )
}

#[cfg(test)]
mod tests;
