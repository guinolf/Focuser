import type { ProtectionPreset } from "@/bindings";
import { m } from "@/paraglide/messages.js";

/**
 * The four profiles as the backend defines them.
 *
 * Kept in step with `ProtectionPreset` in `focuser-common`: the labels are for
 * choosing before a policy exists, but the backend always returns the stored
 * policy, which is what the UI then renders.
 */
export interface PresetMeta {
  id: ProtectionPreset;
  label: () => string;
  hint: () => string;
  /** Effective wait after the 600 s floor. */
  waitSecs: number;
  uninstall: boolean;
  serviceStop: boolean;
  modification: boolean;
}

/** One length for every typing challenge, matching the backend's policy. */
export const CHALLENGE_LENGTH = 20;

/** The backend refuses anything shorter than ten minutes. */
export const MIN_WAIT_SECS = 600;

export const PRESETS: readonly PresetMeta[] = [
  {
    id: "light",
    label: m.protection_preset_light,
    hint: m.protection_preset_light_hint,
    waitSecs: 600,
    uninstall: false,
    serviceStop: false,
    modification: false,
  },
  {
    id: "moderate",
    label: m.protection_preset_moderate,
    hint: m.protection_preset_moderate_hint,
    waitSecs: 600,
    uninstall: false,
    serviceStop: true,
    modification: true,
  },
  {
    id: "firm",
    label: m.protection_preset_firm,
    hint: m.protection_preset_firm_hint,
    waitSecs: 600,
    uninstall: true,
    serviceStop: true,
    modification: true,
  },
  {
    id: "closed",
    label: m.protection_preset_closed,
    hint: m.protection_preset_closed_hint,
    waitSecs: 3600,
    uninstall: true,
    serviceStop: true,
    modification: true,
  },
];

export function presetMeta(id: ProtectionPreset): PresetMeta {
  return PRESETS.find((preset) => preset.id === id) ?? PRESETS[2];
}
