export function initComposer(root: HTMLElement): () => void {
    const form =
        root instanceof HTMLFormElement ? root : root.querySelector("form");
    if (!form) {
        return () => {};
    }

    const onKeyDown = (event: KeyboardEvent) => {
        if (event.isComposing) {
            return;
        }
        if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) {
            return;
        }
        if (!(event.target instanceof HTMLTextAreaElement)) {
            return;
        }
        event.preventDefault();
        form.requestSubmit();
    };

    form.addEventListener("keydown", onKeyDown);
    return () => form.removeEventListener("keydown", onKeyDown);
}
