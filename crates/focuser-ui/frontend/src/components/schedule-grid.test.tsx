import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { CellKey } from "@/lib/schedule";
import { ScheduleGrid } from "./schedule-grid";

function GridWrapper({
  initial = new Set<CellKey>(),
  disabled = false,
}: {
  initial?: Set<CellKey>;
  disabled?: boolean;
}) {
  const [selected, setSelected] = useState<Set<CellKey>>(initial);
  return <ScheduleGrid selected={selected} onChange={setSelected} disabled={disabled} />;
}

describe("ScheduleGrid accessibility & activation", () => {
  it("toggles cell via Enter key without double activation", async () => {
    render(<GridWrapper />);
    const cell = screen.getByRole("button", { name: "Mon 9am" });
    expect(cell).toHaveAttribute("aria-pressed", "false");

    cell.focus();
    fireEvent.keyDown(cell, { key: "Enter" });
    expect(cell).toHaveAttribute("aria-pressed", "true");

    fireEvent.keyDown(cell, { key: "Enter" });
    expect(cell).toHaveAttribute("aria-pressed", "false");
  });

  it("toggles cell via Space key", async () => {
    render(<GridWrapper />);
    const cell = screen.getByRole("button", { name: "Tue 10am" });
    expect(cell).toHaveAttribute("aria-pressed", "false");

    cell.focus();
    fireEvent.keyDown(cell, { key: " " });
    expect(cell).toHaveAttribute("aria-pressed", "true");
  });

  it("prevents double activation when pointerdown is followed by click", async () => {
    render(<GridWrapper />);
    const cell = screen.getByRole("button", { name: "Wed 2pm" });
    expect(cell).toHaveAttribute("aria-pressed", "false");

    // Pointer down activates cell
    fireEvent.pointerDown(cell);
    expect(cell).toHaveAttribute("aria-pressed", "true");

    // Immediate click event (browser synthesized) must not toggle back
    fireEvent.click(cell);
    expect(cell).toHaveAttribute("aria-pressed", "true");
  });

  it("does not toggle when grid is disabled", () => {
    render(<GridWrapper disabled />);
    const cell = screen.getByRole("button", { name: "Thu 11am" });
    expect(cell).toBeDisabled();

    fireEvent.keyDown(cell, { key: "Enter" });
    expect(cell).toHaveAttribute("aria-pressed", "false");

    fireEvent.pointerDown(cell);
    expect(cell).toHaveAttribute("aria-pressed", "false");
  });
});
