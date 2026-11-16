import { CalendarClock, Pause, Play, Timer, Zap } from "lucide-react";
import { useState } from "react";
import type { BlockList, Plan, PlanMode } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, EmptyState, PageHeader } from "@/components/ui/card";
import { ConfirmButton } from "@/components/ui/confirm-button";
import { InlineError, QueryState } from "@/components/ui/feedback";
import { NumberField } from "@/components/ui/number-field";
import { Page } from "@/components/ui/page";
import { Select } from "@/components/ui/select";
import {
  useBlockLists,
  useCancelQuickTimer,
  useEndPlanPause,
  usePlanRuntimes,
  useSetPlan,
  useStartPlanPause,
  useStartQuickTimer,
} from "@/lib/commands";
import { formatCountdown, formatDuration } from "@/lib/duration";
import { planStatusLabel, planStatusTone } from "@/lib/plan-labels";
import { m } from "@/paraglide/messages.js";

const MODES: { value: PlanMode; label: () => string }[] = [
  { value: "block", label: m.plans_mode_block },
  { value: "time_limit", label: m.plans_mode_time_limit },
  { value: "session_limit", label: m.plans_mode_session_limit },
  { value: "screen_break", label: m.plans_mode_screen_break },
  { value: "time_goal", label: m.plans_mode_time_goal },
];

function defaultsFor(mode: PlanMode, existing: Plan | null | undefined): Plan {
  const base: Plan = {
    mode,
    pauses: existing?.pauses ?? { allowed_per_day: 1, duration_minutes: 15, min_gap_minutes: 60 },
  };
  switch (mode) {
    case "time_limit":
      return { ...base, time_limit: existing?.time_limit ?? { daily_minutes: 60 } };
    case "session_limit":
      return {
        ...base,
        session_limit: existing?.session_limit ?? { max_sessions: 5, min_gap_minutes: 30 },
      };
    case "screen_break":
      return {
        ...base,
        screen_break: existing?.screen_break ?? { work_minutes: 50, break_minutes: 10 },
      };
    case "time_goal":
      return { ...base, time_goal: existing?.time_goal ?? { daily_minutes: 120 } };
    default:
      return base;
  }
}

export function Plans() {
  const lists = useBlockLists();
  const runtimes = usePlanRuntimes();
  const setPlan = useSetPlan();
  const pause = useStartPlanPause();
  const resume = useEndPlanPause();
  const quick = useStartQuickTimer();
  const cancelQuick = useCancelQuickTimer();
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState<Plan | null>(null);
  const [minutes, setMinutes] = useState(30);

  const byList = new Map((runtimes.data ?? []).map((runtime) => [runtime.block_list_id, runtime]));

  function begin(list: BlockList) {
    setEditing(list.id);
    setDraft(defaultsFor(list.plan?.mode ?? "block", list.plan));
  }

  function save(list: BlockList) {
    if (!draft) return;
    setPlan.mutate(
      {
        listId: list.id,
        plan: draft,
      },
      {
        onSuccess: () => {
          setEditing(null);
          setDraft(null);
        },
      },
    );
  }

  function removePlan(list: BlockList) {
    setPlan.mutate(
      { listId: list.id, plan: null },
      {
        onSuccess: () => {
          setEditing(null);
          setDraft(null);
        },
      },
    );
  }

  return (
    <Page>
      <PageHeader title={m.plans_title()} description={m.plans_description()} />

      <QueryState
        isPending={lists.isPending || runtimes.isPending}
        error={lists.error ?? runtimes.error}
        onRetry={() => {
          lists.refetch();
          runtimes.refetch();
        }}
        isRetrying={lists.isFetching || runtimes.isFetching}
      >
        {(lists.data ?? []).length === 0 ? (
          <EmptyState
            icon={<CalendarClock />}
            title={m.plans_empty_title()}
            description={m.plans_empty_description()}
          />
        ) : (
          <div className="flex flex-col gap-3">
            {(lists.data ?? []).map((list) => {
              const runtime = byList.get(list.id);
              const current = runtime?.mode ?? list.plan?.mode ?? "block";
              const quickActive =
                runtime?.quick_timer_ends_at != null &&
                new Date(runtime.quick_timer_ends_at).getTime() > Date.now();
              // A quick timer only makes sense where it can arm a blocking
              // mode. Reminder-only plans and lists without a plan get no
              // control that would silently do nothing.
              const canArmQuickly =
                list.plan != null && ["block", "time_limit", "session_limit"].includes(current);
              return (
                <Card key={list.id} padding="md" elevation="raised">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <h2 className="font-medium text-foreground">{list.name}</h2>
                        <Badge tone="neutral">
                          {MODES.find((mode) => mode.value === current)?.label() ?? current}
                        </Badge>
                        {runtime && (
                          <Badge tone={planStatusTone(runtime.status)}>
                            {planStatusLabel(runtime.status)}
                          </Badge>
                        )}
                        {!list.enabled && <Badge tone="neutral">{m.plans_status_inactive()}</Badge>}
                      </div>
                      <p className="mt-1 text-muted-foreground text-xs">
                        {runtime
                          ? runtime.armed
                            ? m.plans_armed()
                            : list.plan
                              ? m.plans_status_inactive()
                              : m.plans_no_plan()
                          : m.plans_no_plan()}
                      </p>
                      {runtime?.paused_until && (
                        <p className="mt-1 text-warning text-xs">
                          {m.plans_pause_until({
                            when: formatCountdown(
                              Math.max(
                                0,
                                (new Date(runtime.paused_until).getTime() - Date.now()) / 1000,
                              ),
                            ),
                          })}
                        </p>
                      )}
                      {quickActive && runtime?.quick_timer_ends_at && (
                        <p className="mt-1 text-primary text-xs">
                          {m.plans_quick_ends({
                            when: formatCountdown(
                              Math.max(
                                0,
                                (new Date(runtime.quick_timer_ends_at).getTime() - Date.now()) /
                                  1000,
                              ),
                            ),
                          })}
                        </p>
                      )}
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      <Button variant="outline" size="sm" onClick={() => begin(list)}>
                        {m.common_edit()}
                      </Button>
                      {runtime?.paused_until ? (
                        <Button
                          variant="soft"
                          size="sm"
                          onClick={() => resume.mutate(list.id)}
                          disabled={resume.isPending}
                        >
                          <Play aria-hidden className="size-3.5" />
                          {m.plans_resume()}
                        </Button>
                      ) : (
                        list.plan?.pauses &&
                        list.plan.pauses.allowed_per_day > 0 &&
                        runtime?.armed === true && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => pause.mutate(list.id)}
                            disabled={pause.isPending}
                          >
                            <Pause aria-hidden className="size-3.5" />
                            {m.plans_pause()}
                          </Button>
                        )
                      )}
                    </div>
                  </div>

                  {runtime && (
                    <div className="mt-3 grid gap-3 text-xs sm:grid-cols-3">
                      <div>
                        <p className="text-faint-foreground">{m.plans_used()}</p>
                        <p className="mt-0.5 font-medium text-foreground tabular-nums">
                          {formatDuration(runtime.used_secs)}
                          {runtime.limit_secs > 0 && ` / ${formatDuration(runtime.limit_secs)}`}
                        </p>
                      </div>
                      <div>
                        <p className="text-faint-foreground">{m.plans_sessions_used()}</p>
                        <p className="mt-0.5 font-medium text-foreground tabular-nums">
                          {runtime.sessions_used}
                          {runtime.sessions_allowed !== null && ` / ${runtime.sessions_allowed}`}
                        </p>
                      </div>
                      <div>
                        <p className="text-faint-foreground">{m.plans_break()}</p>
                        <p className="mt-0.5 font-medium text-foreground">
                          {runtime.break_due_at ? m.plans_break_due() : "—"}
                        </p>
                      </div>
                    </div>
                  )}

                  {canArmQuickly && (
                    <div className="mt-3 flex flex-wrap items-end gap-2 border-border/60 border-t pt-3">
                      <span className="flex items-center gap-1.5 text-muted-foreground text-xs">
                        <Zap aria-hidden className="size-3.5" />
                        {m.plans_quick_timer()}
                      </span>
                      <NumberField
                        value={minutes}
                        onCommit={setMinutes}
                        min={1}
                        max={1440}
                        step={5}
                        suffix={m.common_minutes()}
                        aria-label={m.plans_quick_minutes()}
                      />
                      <Button
                        size="sm"
                        variant="soft"
                        onClick={() => quick.mutate({ listId: list.id, minutes })}
                        disabled={quick.isPending || !list.enabled}
                      >
                        <Timer aria-hidden className="size-3.5" />
                        {m.plans_quick_start()}
                      </Button>
                      {!list.enabled && (
                        <span className="text-faint-foreground text-xs">
                          {m.plans_enable_first()}
                        </span>
                      )}
                      {quickActive && (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => cancelQuick.mutate(list.id)}
                          disabled={cancelQuick.isPending}
                        >
                          {m.plans_quick_cancel()}
                        </Button>
                      )}
                    </div>
                  )}

                  {editing === list.id && draft && (
                    <PlanEditor
                      mode={draft.mode ?? "block"}
                      draft={draft}
                      onDraft={setDraft}
                      onCancel={() => {
                        setEditing(null);
                        setDraft(null);
                      }}
                      onSave={() => save(list)}
                      onRemove={list.plan ? () => removePlan(list) : undefined}
                      pending={setPlan.isPending}
                    />
                  )}
                </Card>
              );
            })}
          </div>
        )}

        <p className="mt-3 text-faint-foreground text-xs">
          {m.plans_limits_block_when_spent()} {m.plans_never_blocks()}
        </p>

        <InlineError
          error={setPlan.error ?? pause.error ?? resume.error ?? quick.error ?? cancelQuick.error}
        />
      </QueryState>
    </Page>
  );
}

function PlanEditor({
  mode,
  draft,
  onDraft,
  onCancel,
  onSave,
  onRemove,
  pending,
}: {
  mode: PlanMode;
  draft: Plan;
  onDraft: (plan: Plan) => void;
  onCancel: () => void;
  onSave: () => void;
  onRemove?: () => void;
  pending: boolean;
}) {
  return (
    <div className="mt-4 rounded-lg border border-border bg-surface/60 p-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-muted-foreground text-xs">{m.plans_mode()}</span>
        <Select
          value={mode}
          onValueChange={(next) => onDraft(defaultsFor(next, draft))}
          options={MODES.map((item) => ({ value: item.value, label: item.label() }))}
          size="sm"
          aria-label={m.plans_mode()}
        />
      </div>

      <div className="mt-3 flex flex-wrap items-end gap-4">
        {mode === "time_limit" && (
          <NumberField
            value={draft.time_limit?.daily_minutes ?? 60}
            onCommit={(daily_minutes) => onDraft({ ...draft, time_limit: { daily_minutes } })}
            min={1}
            max={1440}
            suffix={m.common_minutes()}
            aria-label={m.plans_daily_limit()}
          />
        )}
        {mode === "session_limit" && (
          <>
            <NumberField
              value={draft.session_limit?.max_sessions ?? 5}
              onCommit={(max_sessions) =>
                onDraft({
                  ...draft,
                  session_limit: {
                    max_sessions,
                    min_gap_minutes: draft.session_limit?.min_gap_minutes ?? 30,
                  },
                })
              }
              min={1}
              max={100}
              aria-label={m.plans_sessions_limit()}
            />
            <NumberField
              value={draft.session_limit?.min_gap_minutes ?? 30}
              onCommit={(min_gap_minutes) =>
                onDraft({
                  ...draft,
                  session_limit: {
                    max_sessions: draft.session_limit?.max_sessions ?? 5,
                    min_gap_minutes,
                  },
                })
              }
              min={1}
              max={1440}
              suffix={m.common_minutes()}
              aria-label={m.plans_session_gap()}
            />
          </>
        )}
        {mode === "screen_break" && (
          <>
            <NumberField
              value={draft.screen_break?.work_minutes ?? 50}
              onCommit={(work_minutes) =>
                onDraft({
                  ...draft,
                  screen_break: {
                    work_minutes,
                    break_minutes: draft.screen_break?.break_minutes ?? 10,
                  },
                })
              }
              min={5}
              max={240}
              suffix={m.common_minutes()}
              aria-label={m.plans_work()}
            />
            <NumberField
              value={draft.screen_break?.break_minutes ?? 10}
              onCommit={(break_minutes) =>
                onDraft({
                  ...draft,
                  screen_break: {
                    work_minutes: draft.screen_break?.work_minutes ?? 50,
                    break_minutes,
                  },
                })
              }
              min={1}
              max={120}
              suffix={m.common_minutes()}
              aria-label={m.plans_break()}
            />
          </>
        )}
        {mode === "time_goal" && (
          <NumberField
            value={draft.time_goal?.daily_minutes ?? 120}
            onCommit={(daily_minutes) => onDraft({ ...draft, time_goal: { daily_minutes } })}
            min={1}
            max={1440}
            suffix={m.common_minutes()}
            aria-label={m.plans_target()}
          />
        )}
      </div>

      <fieldset className="mt-4 border-border/60 border-t pt-3">
        <legend className="text-muted-foreground text-xs">{m.plans_pauses()}</legend>
        <div className="mt-2 flex flex-wrap items-end gap-4">
          <NumberField
            value={draft.pauses?.allowed_per_day ?? 1}
            onCommit={(allowed_per_day) =>
              onDraft({
                ...draft,
                pauses: {
                  allowed_per_day,
                  duration_minutes: draft.pauses?.duration_minutes ?? 15,
                  min_gap_minutes: draft.pauses?.min_gap_minutes ?? 60,
                },
              })
            }
            min={0}
            max={12}
            aria-label={m.plans_pause_per_day()}
          />
          <NumberField
            value={draft.pauses?.duration_minutes ?? 15}
            onCommit={(duration_minutes) =>
              onDraft({
                ...draft,
                pauses: {
                  allowed_per_day: draft.pauses?.allowed_per_day ?? 1,
                  duration_minutes,
                  min_gap_minutes: draft.pauses?.min_gap_minutes ?? 60,
                },
              })
            }
            min={1}
            max={120}
            suffix={m.common_minutes()}
            aria-label={m.plans_pause_duration()}
          />
          <NumberField
            value={draft.pauses?.min_gap_minutes ?? 60}
            onCommit={(min_gap_minutes) =>
              onDraft({
                ...draft,
                pauses: {
                  allowed_per_day: draft.pauses?.allowed_per_day ?? 1,
                  duration_minutes: draft.pauses?.duration_minutes ?? 15,
                  min_gap_minutes,
                },
              })
            }
            min={0}
            max={1440}
            suffix={m.common_minutes()}
            aria-label={m.plans_pause_gap()}
          />
        </div>
        <p className="mt-1.5 text-faint-foreground text-xs">{m.plans_pauses_hint()}</p>
      </fieldset>

      <div className="mt-3 flex items-center gap-2">
        <Button size="sm" onClick={onSave} disabled={pending}>
          {m.plans_save()}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>
          {m.common_cancel()}
        </Button>
        {onRemove && (
          <ConfirmButton size="sm" variant="ghost" onConfirm={onRemove} disabled={pending}>
            {m.plans_remove()}
          </ConfirmButton>
        )}
      </div>
    </div>
  );
}
