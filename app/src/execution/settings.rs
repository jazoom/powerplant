use crate::{agents::ToolId, providers::ModelSelection};

pub(crate) const MAXIMUM_INSTRUCTION_BYTES: usize = crate::agents::MAXIMUM_INSTRUCTION_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionSettings {
    pub(crate) model: ModelSelection,
    pub(crate) instructions: String,
    pub(crate) tools: Vec<ToolId>,
}

impl ExecutionSettings {
    pub(crate) fn new(
        model: ModelSelection,
        instructions: String,
        tools: Vec<ToolId>,
    ) -> Option<Self> {
        if instructions.len() > MAXIMUM_INSTRUCTION_BYTES
            || instructions
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
            || tools.len() > ToolId::ALL.len()
            || tools
                .iter()
                .enumerate()
                .any(|(index, tool)| tools[..index].contains(tool))
        {
            return None;
        }
        Some(Self {
            model,
            instructions,
            tools,
        })
    }
}

#[cfg(test)]
mod tests;
