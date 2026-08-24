export function initComposer(root: HTMLElement): () => void {
    const form =
        root instanceof HTMLFormElement ? root : root.querySelector("form");
    const textarea = root.querySelector("textarea");
    if (!form || !textarea) {
        return () => {};
    }

    const onKeyDown = (event: KeyboardEvent) => {
        if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) {
            return;
        }
        event.preventDefault();
        form.requestSubmit();
    };

    textarea.addEventListener("keydown", onKeyDown);
    return () => textarea.removeEventListener("keydown", onKeyDown);
}
