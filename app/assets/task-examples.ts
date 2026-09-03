import type { IslandMountContext } from "hypergraft/browser";

const MAXIMUM_EXAMPLE_LENGTH = 32_768;

function selectedExample(button: HTMLElement): string | null {
    const value = button.dataset.taskExample ?? "";
    if (value === "" || value.length > MAXIMUM_EXAMPLE_LENGTH) {
        return null;
    }
    return value;
}

export function initTaskExamples(
    root: HTMLElement,
    { signal }: IslandMountContext,
): void {
    root.addEventListener(
        "click",
        (event) => {
            if (!(event.target instanceof Element)) {
                return;
            }
            const button = event.target.closest<HTMLButtonElement>(
                'button[type="button"][data-task-example]',
            );
            if (!button || !root.contains(button)) {
                return;
            }
            const text = selectedExample(button);
            if (text === null) {
                return;
            }
            const message =
                document.querySelector<HTMLTextAreaElement>(
                    "#composer-message",
                );
            if (!message) {
                return;
            }
            message.value = text;
            message.focus();
            if (document.activeElement === message) {
                message.setSelectionRange(text.length, text.length);
            }
            message.dispatchEvent(new Event("input", { bubbles: true }));
        },
        { signal },
    );
}
