import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

export function initWorkspace(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance {
    const mobile = matchMedia("(max-width: 700px)");
    let conversation: Element | null = null;
    let workOpen = false;
    let expanded = false;
    let returnFocus: HTMLElement | null = null;
    let previousActivity = false;
    let previousPlan = false;
    let plansReturnFocus: HTMLElement | null = null;
    let setupReturnFocus: HTMLElement | null = null;
    let header: HTMLElement | null = null;
    const positionPanels = () => {
        if (header)
            root.style.setProperty(
                "--workspace-panel-top",
                `${header.getBoundingClientRect().bottom}px`,
            );
    };
    const headerResize = new ResizeObserver(positionPanels);

    function sync() {
        root.querySelectorAll<HTMLElement>(".prose pre").forEach((pre) => {
            pre.tabIndex = 0;
            pre.setAttribute("role", "region");
            pre.setAttribute("aria-label", "Code block");
        });
        root.querySelectorAll<HTMLElement>(
            ".workspace-diff code:not([data-coloured])",
        ).forEach((code) => {
            code.dataset.coloured = "true";
            const lines = (code.textContent ?? "").split("\n");
            if (lines.length > 1000) return;
            // Diff contents are untrusted file text, never HTML.
            code.replaceChildren(
                ...lines.map((text, index) => {
                    const line = document.createElement("span");
                    if (text.startsWith("+") && !text.startsWith("+++"))
                        line.className = "workspace-diff-add";
                    else if (text.startsWith("-") && !text.startsWith("---"))
                        line.className = "workspace-diff-remove";
                    line.textContent =
                        text + (index < lines.length - 1 ? "\n" : "");
                    return line;
                }),
            );
        });
        const detail = root.querySelector<HTMLElement>("#conversation-detail");
        const nextHeader =
            detail?.querySelector<HTMLElement>(":scope > header") ?? null;
        if (nextHeader !== header) {
            headerResize.disconnect();
            header = nextHeader;
            if (header) headerResize.observe(header);
        }
        positionPanels();
        const work = root.querySelector<HTMLElement>("#conversation-work");
        if (detail !== conversation) {
            conversation = detail;
            workOpen = work?.dataset.workActive === "true";
            expanded = false;
            previousActivity = workOpen;
        }
        const active = work?.dataset.workActive === "true";
        const plan = !!root.querySelector("#plan-detail, #workflow-detail");
        if (previousActivity && !active && work?.dataset.workEmpty === "true")
            workOpen = false;
        if (previousPlan && !plan && !active) workOpen = false;
        previousPlan = plan;
        if (active && !previousActivity) workOpen = true;
        previousActivity = active;
        if (work) work.hidden = !workOpen;
        const setup = root.querySelector(
            "#conversation-settings:popover-open, #conversation-documents:popover-open",
        );
        if (work) work.inert = !!setup;
        detail?.toggleAttribute(
            "data-review-expanded",
            expanded && workOpen && !setup,
        );
        const excludeConversation =
            (mobile.matches && (workOpen || !!setup)) || expanded;
        for (const selector of [
            "#transcript",
            "#conversation-composer-dock",
            ".workspace-effective",
            ".workspace-toolbar",
        ]) {
            const element = detail?.querySelector<HTMLElement>(selector);
            if (element) element.inert = excludeConversation;
        }
        root.querySelector("[data-work-toggle]")?.setAttribute(
            "aria-expanded",
            String(workOpen),
        );
        root.querySelectorAll<HTMLAnchorElement>(
            "[data-recent-conversation]",
        ).forEach((link) => {
            const owner = root.querySelector<HTMLElement>(
                "[data-conversation-url]",
            )?.dataset.conversationUrl;
            if (link.pathname === (owner || location.pathname))
                link.setAttribute("aria-current", "page");
            else link.removeAttribute("aria-current");
        });
    }

    function closeWork() {
        workOpen = false;
        expanded = false;
        sync();
        const destination = returnFocus?.isConnected
            ? returnFocus
            : root.querySelector<HTMLElement>(
                  root.querySelector("#workflow-detail")
                      ? "[data-workflow-toggle]"
                      : root.querySelector("#plan-detail")
                        ? "[data-plans-toggle]"
                        : "[data-work-toggle]",
              );
        destination?.focus();
    }

    root.addEventListener(
        "click",
        (event) => {
            const target =
                event.target instanceof Element ? event.target : null;
            const setupTrigger = target?.closest<HTMLElement>(
                '[popovertarget="conversation-settings"]',
            );
            if (
                setupTrigger &&
                setupTrigger.getAttribute("popovertargetaction") !== "hide"
            )
                setupReturnFocus = setupTrigger;
            if (
                target?.closest("[data-plans-toggle]") &&
                !event.ctrlKey &&
                !event.metaKey &&
                !event.shiftKey &&
                !event.altKey &&
                event.button === 0
            ) {
                event.preventDefault();
                plansReturnFocus = target.closest<HTMLElement>(
                    "[data-plans-toggle]",
                );
                root.querySelector<HTMLElement>(
                    "#conversation-documents",
                )?.showPopover();
                sync();
            } else if (target?.closest("[data-work-toggle]")) {
                returnFocus = target.closest<HTMLElement>("button");
                workOpen = !workOpen;
                root.querySelector<HTMLElement>(
                    "#conversation-settings:popover-open, #conversation-documents:popover-open",
                )?.hidePopover();
                sync();
                if (workOpen)
                    root.querySelector<HTMLElement>(
                        "#conversation-work",
                    )?.focus();
            } else if (target?.closest("[data-work-close]")) closeWork();
            else if (
                target?.closest("[data-review-file]") &&
                !event.ctrlKey &&
                !event.metaKey &&
                !event.shiftKey &&
                !event.altKey &&
                event.button === 0
            ) {
                event.preventDefault();
                const selected =
                    target.closest<HTMLElement>("[data-review-file]")?.dataset
                        .reviewFile;
                root.querySelectorAll<HTMLElement>(
                    "[data-review-diff]",
                ).forEach((diff) => {
                    diff.hidden = diff.dataset.reviewDiff !== selected;
                });
                root.querySelectorAll<HTMLElement>(
                    "[data-review-file]",
                ).forEach((link) => {
                    if (link.dataset.reviewFile === selected)
                        link.setAttribute("aria-current", "true");
                    else link.removeAttribute("aria-current");
                });
            } else if (
                target?.closest("[data-expand-review]") &&
                !event.ctrlKey &&
                !event.metaKey &&
                !event.shiftKey &&
                !event.altKey &&
                event.button === 0
            ) {
                event.preventDefault();
                expanded = !expanded;
                const link = target.closest("a");
                if (link)
                    link.textContent = expanded
                        ? "Return to conversation"
                        : "Expand review";
                sync();
            }
            const menu = root.querySelector<HTMLElement>("#workspace-index");
            if (target?.closest("[data-workspace-menu]")) {
                menu?.toggleAttribute("data-menu-open");
                root.querySelector("[data-workspace-menu]")?.setAttribute(
                    "aria-expanded",
                    String(menu?.hasAttribute("data-menu-open")),
                );
            } else if (target?.closest("a[data-graft]")) {
                menu?.removeAttribute("data-menu-open");
                root.querySelector("[data-workspace-menu]")?.setAttribute(
                    "aria-expanded",
                    "false",
                );
            }
        },
        { signal },
    );
    root.addEventListener(
        "keydown",
        (event) => {
            if (
                event.key === "Escape" &&
                !root.querySelector(":popover-open, dialog[open]")
            ) {
                const menu = root.querySelector(
                    "#workspace-index[data-menu-open]",
                );
                if (menu) {
                    menu.removeAttribute("data-menu-open");
                    const trigger = root.querySelector<HTMLElement>(
                        "[data-workspace-menu]",
                    );
                    trigger?.setAttribute("aria-expanded", "false");
                    trigger?.focus();
                } else if (workOpen) closeWork();
            }
        },
        { signal },
    );
    root.addEventListener(
        "toggle",
        (event) => {
            if (
                event.target instanceof HTMLElement &&
                (event.target.id === "conversation-settings" ||
                    event.target.id === "conversation-documents")
            ) {
                sync();
                if (
                    !event.target.matches(":popover-open") &&
                    !root.querySelector(
                        "#conversation-settings:popover-open, #conversation-documents:popover-open",
                    )
                ) {
                    const plans = event.target.id === "conversation-documents";
                    const trigger = plans ? plansReturnFocus : setupReturnFocus;
                    const destination = trigger?.isConnected
                        ? trigger
                        : root.querySelector<HTMLElement>(
                              plans
                                  ? "[data-plans-toggle]"
                                  : '[popovertarget="conversation-settings"]',
                          );
                    destination?.focus();
                }
            }
        },
        { capture: true, signal },
    );
    mobile.addEventListener("change", sync, { signal });
    sync();
    return {
        reconcile: sync,
        destroy() {
            headerResize.disconnect();
            root.style.removeProperty("--workspace-panel-top");
            root.querySelectorAll<HTMLElement>("[inert]").forEach((element) => {
                element.inert = false;
            });
        },
    };
}
