import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { expect, it } from "vitest";
import {
  methodFromLock,
  methodReady,
  toLockSetup,
  type UnlockMethod,
  UnlockMethodPicker,
} from "./unlock-method-picker";

/** The picker with the state a form would hold, and what that form would send. */
function Harness({ start }: { start?: UnlockMethod }) {
  const [method, setMethod] = useState(start ?? methodFromLock(undefined));
  return (
    <>
      <UnlockMethodPicker value={method} onChange={setMethod} />
      <output data-testid="sends">{JSON.stringify(toLockSetup(method))}</output>
      <output data-testid="ready">{String(methodReady(method))}</output>
    </>
  );
}

const sends = () => JSON.parse(screen.getByTestId("sends").textContent ?? "null");
const ready = () => screen.getByTestId("ready").textContent === "true";

it("uses the fixed 20-character challenge with no length knob", () => {
  render(<Harness />);

  expect(screen.getByRole("radio", { name: "Type random text" })).toBeChecked();
  expect(screen.queryByRole("spinbutton")).toBeNull();
  expect(sends()).toEqual({ kind: "random_text", length: 20 });
  expect(ready()).toBe(true);
});

it("is not ready with a password until both fields match", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("radio", { name: "Password" }));

  expect(ready()).toBe(false);
  fireEvent.change(screen.getByPlaceholderText("Choose a password"), {
    target: { value: "secret" },
  });
  fireEvent.change(screen.getByPlaceholderText("Confirm password"), {
    target: { value: "secre" },
  });
  expect(ready()).toBe(false);
  expect(screen.getByText("Passwords don't match")).toBeVisible();

  fireEvent.change(screen.getByPlaceholderText("Confirm password"), {
    target: { value: "secret" },
  });
  expect(ready()).toBe(true);
  expect(sends()).toEqual({ kind: "password", password: "secret" });
});

it("sends no lock for not unlocking at all", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("radio", { name: "None — wait it out" }));

  expect(sends()).toBeNull();
  expect(ready()).toBe(true);
  // The length only belongs to the random text.
  expect(screen.queryByRole("spinbutton")).toBeNull();
});

it("keeps an existing typing lock on the same fixed challenge", () => {
  render(<Harness start={methodFromLock({ RandomText: { length: 32 } })} />);
  expect(screen.queryByRole("spinbutton")).toBeNull();
  expect(sends()).toEqual({ kind: "random_text", length: 20 });
});

it("asks for a saved password again instead of showing it", () => {
  render(<Harness start={methodFromLock({ Password: { hash: "argon2-hash" } })} />);

  expect(screen.getByRole("radio", { name: "Password" })).toBeChecked();
  expect(screen.getByPlaceholderText("Choose a password")).toHaveValue("");
  expect(ready()).toBe(false);
});

it("reads a lock with no method as not unlocking at all", () => {
  render(<Harness start={methodFromLock(null)} />);

  expect(screen.getByRole("radio", { name: "None — wait it out" })).toBeChecked();
});
