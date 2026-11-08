import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { MessageSender } from "./MessageSender";

it("shows a helper's name with no session or agent", () => {
  const { container } = render(<MessageSender origin={{ name: "waker", role: "helper" }} />);
  expect(screen.getByText("waker")).toBeTruthy();
  expect(container.querySelector(".sv-message-sender-icon svg")).not.toBeNull();
});

it("shows a session sender's name", () => {
  render(<MessageSender origin={{ sessionId: "s", name: "Other", agent: "codex", role: "session" }} />);
  expect(screen.getByText("Other")).toBeTruthy();
});
