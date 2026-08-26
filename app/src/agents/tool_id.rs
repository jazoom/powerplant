#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolId {
    List,
    Read,
    Write,
    Run,
}

impl ToolId {
    pub(crate) const ALL: [Self; 4] = [Self::List, Self::Read, Self::Write, Self::Run];

    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "list" => Some(Self::List),
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "run" => Some(Self::Run),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read => "read",
            Self::Write => "write",
            Self::Run => "run",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::List => "List",
            Self::Read => "Read",
            Self::Write => "Write",
            Self::Run => "Run",
        }
    }

    pub(crate) fn needs_write(self) -> bool {
        self == Self::Write
    }
}
