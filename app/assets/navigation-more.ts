import type { IslandMountContext } from "hypergraft/browser";

export function initNavigationMore(
    root: HTMLElement,
    { signal }: IslandMountContext,
): void {
    if (!(root instanceof HTMLDetailsElement)) {
        return;
    }

    root.addEventListener(
        "click",
        (event) => {
            if (!(event.target instanceof Element)) {
                return;
            }
            const destination =
                event.target.closest<HTMLAnchorElement>("a[href]");
            if (destination && root.contains(destination)) {
                root.open = false;
            }
        },
        { signal },
    );
}
