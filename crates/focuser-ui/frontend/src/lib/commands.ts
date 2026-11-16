/**
 * Typed command helpers and their TanStack Query bindings.
 *
 * `Command`, `CommandResult`, and every domain type are **generated** from Rust
 * by tauri-specta into `src/bindings.ts`. Nothing here restates a backend type —
 * change a Rust struct or add a `Command` variant and this file stops compiling
 * until it is updated, which is the whole point.
 */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  ActivityHealth,
  ActivityOverview,
  Allowance,
  AllowanceMatch,
  AllowanceStatus,
  AppMatchType,
  BlockedEvent,
  BlockList,
  BrowserStatus,
  CoachInsight,
  Command,
  CommandResult,
  ExceptionType,
  FocusSession,
  Goal,
  LockSetup,
  Notification,
  Plan,
  PlanRuntime,
  PomodoroConfig,
  PomodoroStatus,
  ProtectionInfo,
  ProtectionPolicy,
  ProtectionPreset,
  Task,
  TimeSlot,
  UnlockChallenge,
  UnlockStatus,
  UsageStat,
  WebsiteMatchType,
  WebsiteRuleKind,
} from "@/bindings";
import { send } from "./transport";

export type {
  ActivityHealth,
  ActivityOverview,
  Allowance,
  AllowanceMatch,
  AllowanceStatus,
  AppMatchType,
  BlockedEvent,
  BlockList,
  BrowserStatus,
  CoachInsight,
  Command,
  CommandResult,
  ExceptionType,
  FocusSession,
  Goal,
  LockSetup,
  Notification,
  Plan,
  PlanRuntime,
  PomodoroConfig,
  PomodoroStatus,
  ProtectionInfo,
  ProtectionPolicy,
  ProtectionPreset,
  Task,
  TimeSlot,
  UnlockChallenge,
  UnlockStatus,
  UsageStat,
  WebsiteMatchType,
  WebsiteRuleKind,
};

/** Narrow a result, failing loudly if the backend sent a different variant. */
function expect<K extends CommandResult["kind"]>(
  result: CommandResult,
  kind: K,
): Extract<CommandResult, { kind: K }> {
  if (result.kind !== kind) {
    throw new Error(`expected a "${kind}" result, backend returned "${result.kind}"`);
  }
  return result as Extract<CommandResult, { kind: K }>;
}

const run = (command: Command) => send<CommandResult>(command);

// ─── Query keys ─────────────────────────────────────────────────────

export const queryKeys = {
  blockLists: ["block-lists"] as const,
  protection: ["protection"] as const,
  scheduledProtection: ["scheduled-protection"] as const,
  sharedAllowance: ["shared-allowance"] as const,
  allowances: ["allowances"] as const,
  pomodoro: ["pomodoro"] as const,
  stats: (from: string, to: string) => ["stats", from, to] as const,
  events: (from: string, to: string) => ["events", from, to] as const,
  retention: ["stats-retention"] as const,
  activity: (from: string, to: string) => ["activity", from, to] as const,
  activityHealth: ["activity-health"] as const,
  activityRetention: ["activity-retention"] as const,
  planRuntimes: ["plan-runtimes"] as const,
  protectionPolicy: (listId: string) => ["protection-policy", listId] as const,
  unlockStatus: (listId: string) => ["unlock-status", listId] as const,
  goals: (includeArchived: boolean) => ["goals", includeArchived] as const,
  goalProgress: ["goal-progress"] as const,
  tasks: (goalId: string | null, includeDone: boolean) => ["tasks", goalId, includeDone] as const,
  notifications: (unreadOnly: boolean) => ["notifications", unreadOnly] as const,
  coach: ["coach"] as const,
  focusSessions: (from: string, to: string) => ["focus-sessions", from, to] as const,
  browsers: ["browsers"] as const,
  setting: (key: string) => ["setting", key] as const,
  appIcons: (targets: string[]) => ["app-icons", ...targets] as const,
};

/** Anything that changes blocking invalidates lists and protection together. */
function useBlockListMutation<TArgs>(build: (args: TArgs) => Command) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: TArgs) => run(build(args)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.blockLists });
      qc.invalidateQueries({ queryKey: queryKeys.protection });
      qc.invalidateQueries({ queryKey: queryKeys.scheduledProtection });
      qc.invalidateQueries({ queryKey: queryKeys.sharedAllowance });
      qc.invalidateQueries({ queryKey: queryKeys.allowances });
    },
  });
}

// ─── Block lists ────────────────────────────────────────────────────

export function useBlockLists() {
  return useQuery({
    queryKey: queryKeys.blockLists,
    queryFn: async () => expect(await run({ cmd: "list_block_lists" }), "block_lists").data,
  });
}

export function useCreateBlockList() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) =>
      expect(await run({ cmd: "create_block_list", args: { name } }), "block_list").data,
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.blockLists }),
  });
}

export const useToggleBlockList = () =>
  useBlockListMutation<{ id: string; enabled: boolean }>((args) => ({
    cmd: "toggle_block_list",
    args,
  }));

export const useDeleteBlockList = () =>
  useBlockListMutation<string>((id) => ({ cmd: "delete_block_list", args: { id } }));

export const useUpdateBlockList = () =>
  useBlockListMutation<BlockList>((list) => ({ cmd: "update_block_list", args: { list } }));

// ─── Website rules ──────────────────────────────────────────────────

export const useAddWebsiteRule = () =>
  useBlockListMutation<{ listId: string; rule: WebsiteMatchType }>(({ listId, rule }) => ({
    cmd: "add_website_rule",
    args: { list_id: listId, rule },
  }));

export const useRemoveWebsiteRule = () =>
  useBlockListMutation<{ listId: string; ruleId: string }>(({ listId, ruleId }) => ({
    cmd: "remove_website_rule",
    args: { list_id: listId, rule_id: ruleId },
  }));

export const useBulkImportWebsites = () =>
  useBlockListMutation<{ listId: string; values: string[]; kind: WebsiteRuleKind }>(
    ({ listId, values, kind }) => ({
      cmd: "bulk_import_websites",
      args: { list_id: listId, values, kind },
    }),
  );

export const useClearAllWebsites = () =>
  useBlockListMutation<void>(() => ({ cmd: "clear_all_websites" }));

// ─── Application rules ──────────────────────────────────────────────

export const useAddAppRule = () =>
  useBlockListMutation<{ listId: string; rule: AppMatchType }>(({ listId, rule }) => ({
    cmd: "add_app_rule",
    args: { list_id: listId, rule },
  }));

export const useRemoveAppRule = () =>
  useBlockListMutation<{ listId: string; ruleId: string }>(({ listId, ruleId }) => ({
    cmd: "remove_app_rule",
    args: { list_id: listId, rule_id: ruleId },
  }));

export const useClearAllApps = () => useBlockListMutation<void>(() => ({ cmd: "clear_all_apps" }));

// ─── Exceptions ─────────────────────────────────────────────────────

export const useAddException = () =>
  useBlockListMutation<{ listId: string; exception: ExceptionType }>(({ listId, exception }) => ({
    cmd: "add_exception",
    args: { list_id: listId, exception },
  }));

export const useRemoveException = () =>
  useBlockListMutation<{ listId: string; exceptionId: string }>(({ listId, exceptionId }) => ({
    cmd: "remove_exception",
    args: { list_id: listId, exception_id: exceptionId },
  }));

// ─── Schedule ───────────────────────────────────────────────────────

export const useUpdateSchedule = () =>
  useBlockListMutation<{ listId: string; slots: TimeSlot[]; alwaysActive: boolean }>(
    ({ listId, slots, alwaysActive }) => ({
      cmd: "update_schedule",
      args: { list_id: listId, slots, always_active: alwaysActive },
    }),
  );

// ─── Statistics ─────────────────────────────────────────────────────

export function useStats(from: string, to: string) {
  return useQuery({
    queryKey: queryKeys.stats(from, to),
    queryFn: async () => expect(await run({ cmd: "get_stats", args: { from, to } }), "stats").data,
  });
}

export function useBlockedEvents(from: string, to: string) {
  return useQuery({
    queryKey: queryKeys.events(from, to),
    queryFn: async () =>
      expect(await run({ cmd: "get_blocked_events", args: { from, to } }), "blocked_events").data,
  });
}

export function useStatsRetention() {
  return useQuery({
    queryKey: queryKeys.retention,
    queryFn: async () => expect(await run({ cmd: "get_stats_retention" }), "count").data,
  });
}

export function useSetStatsRetention() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (days: number) => run({ cmd: "set_stats_retention", args: { days } }),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.retention }),
  });
}

export function useClearStatistics() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "clear_statistics" }),
    // Keyed by date range, so invalidate by prefix.
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["stats"] });
      qc.invalidateQueries({ queryKey: ["events"] });
    },
  });
}

// ─── Whole configuration ────────────────────────────────────────────

/** The exported document as text. Where it is written is the caller's problem. */
export function useExportConfiguration() {
  return useMutation({
    mutationFn: async () => expect(await run({ cmd: "export_configuration" }), "text").data,
  });
}

export function useImportConfiguration() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (json: string) =>
      expect(await run({ cmd: "import_configuration", args: { json } }), "count").data,
    onSuccess: () => qc.invalidateQueries(),
  });
}

/** Wipes everything, so every cached query is stale afterwards. */
export function useDeleteAllData() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "delete_all_data" }),
    onSuccess: () => qc.invalidateQueries(),
  });
}

// ─── Diagnostics ────────────────────────────────────────────────────

export function useBrowserStatus() {
  return useQuery({
    queryKey: queryKeys.browsers,
    queryFn: async () => expect(await run({ cmd: "get_browser_status" }), "browser_status").data,
    // Browsers start and stop while the window is open, but not every 2s.
    refetchInterval: 10_000,
  });
}

/**
 * Real icons for application rules, keyed by the rule's value.
 *
 * An executable's icon only changes when the program is reinstalled, so this
 * never goes stale on its own — otherwise the Applications page would re-read
 * every icon off disk on each poll. The app-wide poll ignores staleTime, so it
 * is switched off here too; on Linux each read walks the icon themes (#59).
 */
export function useAppIcons(targets: string[]) {
  const key = [...targets].sort();

  return useQuery({
    queryKey: queryKeys.appIcons(key),
    enabled: key.length > 0,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    refetchInterval: false,
    queryFn: async () => {
      const icons = expect(
        await run({ cmd: "get_app_icons", args: { targets: key } }),
        "app_icons",
      ).data;
      return new Map(icons.map((i) => [i.target, i.data_uri]));
    },
  });
}

export function useAppVersion() {
  return useQuery({
    queryKey: ["app-version"] as const,
    queryFn: async () => expect(await run({ cmd: "app_version" }), "text").data,
    staleTime: Number.POSITIVE_INFINITY,
    refetchInterval: false,
  });
}

// ─── Protection ─────────────────────────────────────────────────────

export function useProtectionStatus() {
  return useQuery({
    queryKey: queryKeys.protection,
    queryFn: async () =>
      expect(await run({ cmd: "get_protection_status" }), "protection_status").data,
  });
}

export function useScheduledProtectionStatus() {
  return useQuery({
    queryKey: queryKeys.scheduledProtection,
    queryFn: async () =>
      expect(await run({ cmd: "get_scheduled_protection_status" }), "scheduled_protection_status")
        .data,
  });
}

export function useSharedAllowanceStatus() {
  return useQuery({
    queryKey: queryKeys.sharedAllowance,
    queryFn: async () =>
      expect(await run({ cmd: "get_shared_allowance_status" }), "shared_allowance_status").data,
  });
}

export const useConfigureSharedAllowance = () =>
  useBlockListMutation<{ listId: string; minutes: number | null }>((a) => ({
    cmd: "configure_shared_allowance",
    args: { list_id: a.listId, minutes: a.minutes },
  }));

export const useRelockScheduledProtection = () =>
  useBlockListMutation<string>((listId) => ({
    cmd: "relock_scheduled_protection",
    args: { list_id: listId },
  }));

export const useConfigureScheduledProtection = () =>
  useBlockListMutation<{ listId: string; enabled: boolean; lock: LockSetup | null }>((a) => ({
    cmd: "configure_scheduled_protection",
    args: { list_id: a.listId, enabled: a.enabled, lock: a.lock },
  }));

export const useEnableProtection = () =>
  useBlockListMutation<{
    listId: string;
    /** `null` locks until unlocked; it needs a `lock`. */
    minutes: number | null;
    preventUninstall: boolean;
    preventServiceStop: boolean;
    preventModification: boolean;
    /** `null` means "wait it out" — nothing can end the window early. */
    lock: LockSetup | null;
    /** Named profile that seeds the policy; omitted means the default. */
    preset?: ProtectionPreset | null;
  }>((a) => ({
    cmd: "enable_protection",
    args: {
      list_id: a.listId,
      duration_minutes: a.minutes,
      prevent_uninstall: a.preventUninstall,
      prevent_service_stop: a.preventServiceStop,
      prevent_modification: a.preventModification,
      lock: a.lock,
      preset: a.preset ?? null,
    },
  }));

/** Issue (or reuse) a random-text challenge to display and retype. */
export function useRequestUnlockChallenge() {
  return useMutation({
    mutationFn: async (listId: string) =>
      expect(
        await run({ cmd: "request_unlock_challenge", args: { list_id: listId } }),
        "unlock_challenge",
      ).data,
  });
}

/** Answer a protected list's lock; on success its protection window ends immediately. */
export const useUnlockProtection = () =>
  useBlockListMutation<{ listId: string; response: string }>(({ listId, response }) => ({
    cmd: "unlock_protection",
    args: { list_id: listId, response },
  }));

// ─── Settings ───────────────────────────────────────────────────────

export function useSetting(key: string, fallback?: string) {
  return useQuery({
    queryKey: queryKeys.setting(key),
    queryFn: async () =>
      expect(await run({ cmd: "get_setting", args: { key, default: fallback ?? null } }), "setting")
        .data,
  });
}

export function useSetSetting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { key: string; value: string }) => run({ cmd: "set_setting", args }),
    onSuccess: (_data, { key }) => qc.invalidateQueries({ queryKey: queryKeys.setting(key) }),
  });
}

export function useResetSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "reset_settings" }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["setting"] });
      // Retention lives in the same table and is wiped too, but it has its own key.
      qc.invalidateQueries({ queryKey: queryKeys.retention });
    },
  });
}

// ─── Enforcement ────────────────────────────────────────────────────

export const useApplyBlocks = () => useBlockListMutation<void>(() => ({ cmd: "apply_blocks" }));
export const useRemoveBlocks = () => useBlockListMutation<void>(() => ({ cmd: "remove_blocks" }));

// ─── Pomodoro ───────────────────────────────────────────────────────

export function usePomodoroStatus() {
  return useQuery({
    queryKey: queryKeys.pomodoro,
    queryFn: async () => expect(await run({ cmd: "pomodoro_status" }), "pomodoro_status").data,
    // A countdown that only moves every 2s looks broken; this one ticks.
    refetchInterval: 1000,
  });
}

/**
 * Whether blocking is actually taking effect.
 *
 * Polled rather than fetched once: the extension can connect or drop, and the
 * user can relaunch as administrator, without reloading the app.
 */
export function useBlockingHealth() {
  return useQuery({
    queryKey: ["blocking-health"] as const,
    queryFn: async () => expect(await run({ cmd: "get_blocking_health" }), "blocking_health").data,
    refetchInterval: 5000,
  });
}

/**
 * The built-in duration presets, straight from Rust.
 *
 * They never change while the app is open, so this is fetched once and kept —
 * refetching constants would only add flicker.
 */
export function usePomodoroPresets() {
  return useQuery({
    queryKey: ["pomodoro-presets"] as const,
    queryFn: async () => expect(await run({ cmd: "pomodoro_presets" }), "pomodoro_presets").data,
    staleTime: Number.POSITIVE_INFINITY,
    refetchInterval: false,
  });
}

function usePomodoroMutation<TArgs>(build: (args: TArgs) => Command) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: TArgs) => run(build(args)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.pomodoro });
      // A session toggles its block list, so those change too.
      qc.invalidateQueries({ queryKey: queryKeys.blockLists });
    },
  });
}

export const useStartPomodoro = () =>
  usePomodoroMutation<{ blockListId: string; config: PomodoroConfig; lockRounds?: number }>(
    ({ blockListId, config, lockRounds }) => ({
      cmd: "pomodoro_start",
      args: { block_list_id: blockListId, config, lock_rounds: lockRounds ?? null },
    }),
  );

export const usePausePomodoro = () => usePomodoroMutation<void>(() => ({ cmd: "pomodoro_pause" }));
export const useResumePomodoro = () =>
  usePomodoroMutation<void>(() => ({ cmd: "pomodoro_resume" }));
export const useSkipPomodoro = () => usePomodoroMutation<void>(() => ({ cmd: "pomodoro_skip" }));
export const useStopPomodoro = () => usePomodoroMutation<void>(() => ({ cmd: "pomodoro_stop" }));

export function usePomodoroHistory(days: number) {
  return useQuery({
    queryKey: ["pomodoro-history", days] as const,
    queryFn: async () =>
      expect(await run({ cmd: "pomodoro_history", args: { days } }), "pomodoro_history").data,
  });
}

// ─── Allowances ─────────────────────────────────────────────────────

export function useAllowances() {
  return useQuery({
    queryKey: queryKeys.allowances,
    queryFn: async () => expect(await run({ cmd: "allowance_list" }), "allowances").data,
  });
}

function useAllowanceMutation<TArgs>(build: (args: TArgs) => Command) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: TArgs) => run(build(args)),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.allowances }),
  });
}

export const useCreateAllowance = () =>
  useAllowanceMutation<{ target: AllowanceMatch; dailyLimitSecs: number; strictMode: boolean }>(
    ({ target, dailyLimitSecs, strictMode }) => ({
      cmd: "allowance_create",
      args: { target, daily_limit_secs: dailyLimitSecs, strict_mode: strictMode },
    }),
  );

export const useUpdateAllowance = () =>
  useAllowanceMutation<{
    id: string;
    dailyLimitSecs: number;
    strictMode: boolean;
    enabled: boolean;
  }>((a) => ({
    cmd: "allowance_update",
    args: {
      id: a.id,
      daily_limit_secs: a.dailyLimitSecs,
      strict_mode: a.strictMode,
      enabled: a.enabled,
    },
  }));

export const useDeleteAllowance = () =>
  useAllowanceMutation<string>((id) => ({ cmd: "allowance_delete", args: { id } }));

export const useResetAllowanceToday = () =>
  useAllowanceMutation<string>((id) => ({ cmd: "allowance_reset_today", args: { id } }));

// ─── Activity ───────────────────────────────────────────────────────

export function useActivityOverview(from: string, to: string) {
  return useQuery({
    queryKey: queryKeys.activity(from, to),
    queryFn: async () =>
      expect(await run({ cmd: "get_activity_overview", args: { from, to } }), "activity").data,
  });
}

export function useActivityHealth() {
  return useQuery({
    queryKey: queryKeys.activityHealth,
    queryFn: async () => expect(await run({ cmd: "get_activity_health" }), "activity_health").data,
    refetchInterval: 5000,
  });
}

export function useClearActivity() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "clear_activity" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["activity"] }),
  });
}

export function useActivityRetention() {
  return useQuery({
    queryKey: queryKeys.activityRetention,
    queryFn: async () => expect(await run({ cmd: "get_activity_retention" }), "count").data,
  });
}

export function useSetActivityRetention() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (days: number) => run({ cmd: "set_activity_retention", args: { days } }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.activityRetention });
      qc.invalidateQueries({ queryKey: ["activity"] });
    },
  });
}

// ─── Plans ──────────────────────────────────────────────────────────

export function usePlanRuntimes() {
  return useQuery({
    queryKey: queryKeys.planRuntimes,
    queryFn: async () => expect(await run({ cmd: "get_plan_runtimes" }), "plan_runtimes").data,
    refetchInterval: 5000,
  });
}

function usePlanMutation<TArgs>(build: (args: TArgs) => Command) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: TArgs) => run(build(args)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.planRuntimes });
      qc.invalidateQueries({ queryKey: queryKeys.blockLists });
      qc.invalidateQueries({ queryKey: queryKeys.protection });
    },
  });
}

export const useSetPlan = () =>
  usePlanMutation<{ listId: string; plan: Plan | null }>((a) => ({
    cmd: "set_plan",
    args: { list_id: a.listId, plan: a.plan },
  }));

export const useStartPlanPause = () =>
  usePlanMutation<string>((listId) => ({ cmd: "start_plan_pause", args: { list_id: listId } }));

export const useEndPlanPause = () =>
  usePlanMutation<string>((listId) => ({ cmd: "end_plan_pause", args: { list_id: listId } }));

export const useStartQuickTimer = () =>
  usePlanMutation<{ listId: string; minutes: number }>((a) => ({
    cmd: "start_quick_timer",
    args: { list_id: a.listId, minutes: a.minutes },
  }));

export const useCancelQuickTimer = () =>
  usePlanMutation<string>((listId) => ({
    cmd: "cancel_quick_timer",
    args: { list_id: listId },
  }));

// ─── Protection policy ──────────────────────────────────────────────

export function useProtectionPolicy(listId: string | null) {
  return useQuery({
    queryKey: queryKeys.protectionPolicy(listId ?? ""),
    queryFn: async () =>
      expect(
        await run({ cmd: "get_protection_policy", args: { list_id: listId ?? "" } }),
        "protection_policy",
      ).data,
    enabled: listId !== null,
  });
}

export function useSetProtectionPolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { listId: string; policy: ProtectionPolicy }) =>
      run({ cmd: "set_protection_policy", args: { list_id: a.listId, policy: a.policy } }),
    onSuccess: (_d, a) => {
      qc.invalidateQueries({ queryKey: queryKeys.protectionPolicy(a.listId) });
      qc.invalidateQueries({ queryKey: queryKeys.blockLists });
    },
  });
}

export function useUnlockStatus(listId: string | null) {
  return useQuery({
    queryKey: queryKeys.unlockStatus(listId ?? ""),
    queryFn: async () =>
      expect(
        await run({ cmd: "get_unlock_status", args: { list_id: listId ?? "" } }),
        "unlock_status",
      ).data,
    enabled: listId !== null,
    refetchInterval: 1000,
  });
}

// ─── Goals, tasks, notifications, coach ─────────────────────────────

export function useGoals(includeArchived = false) {
  return useQuery({
    queryKey: queryKeys.goals(includeArchived),
    queryFn: async () =>
      expect(await run({ cmd: "get_goals", args: { include_archived: includeArchived } }), "goals")
        .data,
  });
}

export function useGoalProgress() {
  return useQuery({
    queryKey: queryKeys.goalProgress,
    queryFn: async () => expect(await run({ cmd: "get_goal_progress" }), "goal_progress").data,
  });
}

function useProductivityMutation<TArgs>(build: (args: TArgs) => Command) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: TArgs) => run(build(args)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["goals"] });
      qc.invalidateQueries({ queryKey: queryKeys.goalProgress });
      qc.invalidateQueries({ queryKey: ["tasks"] });
      qc.invalidateQueries({ queryKey: ["notifications"] });
      qc.invalidateQueries({ queryKey: queryKeys.coach });
      qc.invalidateQueries({ queryKey: ["focus-sessions"] });
    },
  });
}

export const useSaveGoal = () =>
  useProductivityMutation<Goal>((goal) => ({ cmd: "save_goal", args: { goal } }));

export const useDeleteGoal = () =>
  useProductivityMutation<string>((id) => ({ cmd: "delete_goal", args: { id } }));

export function useTasks(goalId: string | null = null, includeDone = false) {
  return useQuery({
    queryKey: queryKeys.tasks(goalId, includeDone),
    queryFn: async () =>
      expect(
        await run({ cmd: "get_tasks", args: { goal_id: goalId, include_done: includeDone } }),
        "tasks",
      ).data,
  });
}

export const useSaveTask = () =>
  useProductivityMutation<Task>((task) => ({ cmd: "save_task", args: { task } }));

export const useDeleteTask = () =>
  useProductivityMutation<string>((id) => ({ cmd: "delete_task", args: { id } }));

export function useNotifications(unreadOnly = false) {
  return useQuery({
    queryKey: queryKeys.notifications(unreadOnly),
    queryFn: async () =>
      expect(
        await run({ cmd: "get_notifications", args: { limit: 50, unread_only: unreadOnly } }),
        "notifications",
      ).data,
    refetchInterval: 10000,
  });
}

export const useMarkNotificationRead = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => run({ cmd: "mark_notification_read", args: { id } }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notifications"] }),
  });
};

export const useMarkAllNotificationsRead = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "mark_all_notifications_read" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notifications"] }),
  });
};

export const useClearNotifications = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => run({ cmd: "clear_notifications" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notifications"] }),
  });
};

export function useCoachInsights() {
  return useQuery({
    queryKey: queryKeys.coach,
    queryFn: async () => expect(await run({ cmd: "get_coach_insights" }), "coach_insights").data,
  });
}

export function useFocusSessions(from: string, to: string) {
  return useQuery({
    queryKey: queryKeys.focusSessions(from, to),
    queryFn: async () =>
      expect(await run({ cmd: "get_focus_sessions", args: { from, to } }), "focus_sessions").data,
  });
}

export const useStartFocusSession = () =>
  useProductivityMutation<{ label: string; goalId: string | null; taskId: string | null }>((a) => ({
    cmd: "start_focus_session",
    args: { label: a.label, goal_id: a.goalId, task_id: a.taskId },
  }));

export const useFinishFocusSession = () =>
  useProductivityMutation<string>((id) => ({ cmd: "finish_focus_session", args: { id } }));
