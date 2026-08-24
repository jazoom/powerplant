// @vitest-environment happy-dom
import { expect, test } from "vitest";
import { PIN_THRESHOLD_PX, isPinned, leftoverPx } from "./transcript";

test("leftover is the distance from the viewport bottom to the document end", () => {
    expect(
        leftoverPx({
            scrollHeight: 1000,
            scrollY: 200,
            viewportHeight: 700,
        }),
    ).toBe(100);
});

test("a leftover at the threshold stays pinned", () => {
    expect(isPinned(0)).toBe(true);
    expect(isPinned(PIN_THRESHOLD_PX)).toBe(true);
});

test("a leftover above the threshold is unpinned", () => {
    expect(isPinned(PIN_THRESHOLD_PX + 1)).toBe(false);
});

test("automatic scroll follows only a pinned leftover", () => {
    const beforeAppend = leftoverPx({
        scrollHeight: 800,
        scrollY: 680,
        viewportHeight: 100,
    });
    expect(isPinned(beforeAppend)).toBe(true);

    const afterLargeAppend = leftoverPx({
        scrollHeight: 2000,
        scrollY: 680,
        viewportHeight: 100,
    });
    expect(isPinned(afterLargeAppend)).toBe(false);
    expect(isPinned(beforeAppend)).toBe(true);
});

test("a user who scrolls upward is unpinned", () => {
    const afterScrollUp = leftoverPx({
        scrollHeight: 2000,
        scrollY: 200,
        viewportHeight: 100,
    });
    expect(isPinned(afterScrollUp)).toBe(false);
});
