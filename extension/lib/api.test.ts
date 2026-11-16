import { afterEach, describe, expect, it, vi } from "vitest";

import { sendAllowanceTick } from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("sendAllowanceTick", () => {
  it("reports whether the browser window was focused and how idle the user was", async () => {
    const fetchMock = vi.fn(async () => new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);

    await sendAllowanceTick("example.com", 30, "test", "https://example.com/page", false, {
      windowFocused: false,
      idleSecs: 60,
    });

    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse(String(init?.body));
    expect(body).toMatchObject({
      hostname: "example.com",
      window_focused: false,
      idle_secs: 60,
      active: true,
    });
  });

  it("omits the focus claim when an older caller does not measure it", async () => {
    const fetchMock = vi.fn(async () => new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);

    await sendAllowanceTick("example.com", 5, "test");

    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse(String(init?.body));
    expect(body.window_focused).toBeNull();
    expect(body.idle_secs).toBeNull();
  });
});
