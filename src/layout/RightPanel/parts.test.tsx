import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Section } from "./parts";
import { useTermStore } from "../../store/termStore";

// jsdom has no matchMedia, and toggling a section persists through the store, which resolves the theme.
beforeAll(() => {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
});

afterEach(() => {
  cleanup();
  useTermStore.setState({ infoCollapsed: {} });
});

describe("Section", () => {
  it("collapses and expands from the toggle button", () => {
    render(
      <Section id="model" title="Model">
        <div>model rows</div>
      </Section>,
    );
    const toggle = screen.getByRole("button", { name: "Collapse section" });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(toggle);
    expect(screen.queryByText("model rows")).toBeNull();
    expect(useTermStore.getState().infoCollapsed.model).toBe(true);
    expect(screen.getByRole("button", { name: "Expand section" }).getAttribute("aria-expanded")).toBe(
      "false",
    );

    fireEvent.click(screen.getByRole("button", { name: "Expand section" }));
    expect(screen.getByText("model rows")).toBeTruthy();
    // Expanding drops the key instead of storing `false`, keeping the persisted map sparse.
    expect(useTermStore.getState().infoCollapsed.model).toBeUndefined();
  });

  it("toggles from the caption as well, so the title is a target too", () => {
    render(
      <Section id="usage" title="Usage" actions={<span>badge</span>}>
        <div>usage rows</div>
      </Section>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Usage" }));
    expect(screen.queryByText("usage rows")).toBeNull();
    // Header controls stay reachable while the body is hidden.
    expect(screen.getByText("badge")).toBeTruthy();
  });

  it("keeps a collapsed section collapsed when it is unmounted and mounted again", () => {
    useTermStore.setState({ infoCollapsed: { agent: true } });
    render(
      <Section id="agent" title="Agent">
        <div>agent rows</div>
      </Section>,
    );
    expect(screen.queryByText("agent rows")).toBeNull();
  });
});
