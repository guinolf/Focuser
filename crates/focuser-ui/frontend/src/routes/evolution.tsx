import {
  CheckCircle2,
  Circle,
  Flag,
  ListChecks,
  ListTodo,
  Plus,
  Sparkles,
  Timer,
} from "lucide-react";
import { useMemo, useState } from "react";
import type {
  ChecklistItem,
  Goal,
  GoalKind,
  GoalPeriod,
  Notification,
  Task,
  TaskStatus,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, EmptyState, PageHeader, Section } from "@/components/ui/card";
import { ConfirmButton } from "@/components/ui/confirm-button";
import { InlineError, QueryState } from "@/components/ui/feedback";
import { Input } from "@/components/ui/input";
import { NumberField } from "@/components/ui/number-field";
import { Page } from "@/components/ui/page";
import { Progress } from "@/components/ui/progress";
import { Select } from "@/components/ui/select";
import {
  useBlockLists,
  useCoachInsights,
  useDeleteGoal,
  useDeleteTask,
  useFinishFocusSession,
  useFocusSessions,
  useGoalProgress,
  useGoals,
  useMarkAllNotificationsRead,
  useNotifications,
  useSaveGoal,
  useSaveTask,
  useStartFocusSession,
  useTasks,
} from "@/lib/commands";
import { rangeFor } from "@/lib/date-range";
import { formatDuration } from "@/lib/duration";
import { m } from "@/paraglide/messages.js";

const GOAL_KINDS: { value: GoalKind; label: () => string }[] = [
  { value: "time", label: m.evolution_goal_kind_time },
  { value: "sessions", label: m.evolution_goal_kind_sessions },
  { value: "result", label: m.evolution_goal_kind_result },
];

const GOAL_PERIODS: { value: GoalPeriod; label: () => string }[] = [
  { value: "day", label: m.evolution_period_day },
  { value: "week", label: m.evolution_period_week },
  { value: "month", label: m.evolution_period_month },
  { value: "once", label: m.evolution_period_once },
];

const TASK_STATUSES: { value: TaskStatus; label: () => string }[] = [
  { value: "todo", label: m.evolution_task_todo },
  { value: "doing", label: m.evolution_task_doing },
  { value: "done", label: m.evolution_task_done },
  { value: "archived", label: m.evolution_task_archived },
];

function emptyGoal(): Goal {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    title: "",
    kind: "sessions",
    period: "week",
    target_value: 5,
    block_list_id: null,
    state: "active",
    created_at: now,
    updated_at: now,
  };
}

function emptyTask(): Task {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    title: "",
    status: "todo",
    priority: "medium",
    goal_id: null,
    created_at: now,
    updated_at: now,
  };
}

function notificationKindLabel(kind: Notification["kind"]): string {
  switch (kind) {
    case "break_due":
      return m.notifications_kind_break_due();
    case "quick_timer_ended":
      return m.notifications_kind_quick_timer_ended();
    case "goal_reached":
      return m.notifications_kind_goal_reached();
    case "goal_behind":
      return m.notifications_kind_goal_behind();
    case "allowance_warning":
      return m.notifications_kind_allowance_warning();
    case "allowance_exhausted":
      return m.notifications_kind_allowance_exhausted();
    case "blocking_health":
      return m.notifications_kind_blocking_health();
    case "coach_tip":
      return m.notifications_kind_coach_tip();
    case "unlock_ready":
      return m.notifications_kind_unlock_ready();
    default:
      return kind;
  }
}

function goalStateLabel(state: Goal["state"]): string {
  switch (state) {
    case "completed":
      return m.evolution_goal_state_completed();
    case "archived":
      return m.evolution_goal_state_archived();
    default:
      return m.evolution_goal_state_active();
  }
}

export function Evolution() {
  const goals = useGoals();
  const progress = useGoalProgress();
  const lists = useBlockLists();
  const tasks = useTasks(null, true);
  const coach = useCoachInsights();
  const notifications = useNotifications(true);
  const markAll = useMarkAllNotificationsRead();
  const sessions = useFocusSessions(rangeFor("30d").from, rangeFor("30d").to);

  const saveGoal = useSaveGoal();
  const deleteGoal = useDeleteGoal();
  const saveTask = useSaveTask();
  const deleteTask = useDeleteTask();
  const startFocus = useStartFocusSession();
  const finishFocus = useFinishFocusSession();

  const [goalDraft, setGoalDraft] = useState<Goal | null>(null);
  const [taskTitle, setTaskTitle] = useState("");
  const [taskGoal, setTaskGoal] = useState<string>("");
  const [taskEstimate, setTaskEstimate] = useState(25);
  const [checklistTask, setChecklistTask] = useState<string | null>(null);
  const [checklistText, setChecklistText] = useState("");
  const [focusLabel, setFocusLabel] = useState("");
  const [focusGoal, setFocusGoal] = useState<string>("");

  const progressById = useMemo(
    () => new Map((progress.data ?? []).map((p) => [p.goal.id, p])),
    [progress.data],
  );
  const activeSession = useMemo(
    () => (sessions.data ?? []).find((session) => !session.ended_at) ?? null,
    [sessions.data],
  );
  const listOptions = [
    { value: "", label: m.common_none() },
    ...(lists.data ?? []).map((list) => ({ value: list.id, label: list.name })),
  ];

  function setTaskStatus(task: Task, status: TaskStatus) {
    saveTask.mutate({
      ...task,
      status,
      completed_at: status === "done" ? (task.completed_at ?? new Date().toISOString()) : null,
      updated_at: new Date().toISOString(),
    });
  }

  function addChecklistItem(task: Task) {
    const text = checklistText.trim();
    if (!text) return;
    const item: ChecklistItem = { id: crypto.randomUUID(), text, done: false };
    saveTask.mutate(
      {
        ...task,
        checklist: [...(task.checklist ?? []), item],
        updated_at: new Date().toISOString(),
      },
      { onSuccess: () => setChecklistText("") },
    );
  }

  return (
    <Page wide>
      <PageHeader title={m.evolution_title()} description={m.evolution_description()} />

      <QueryState
        isPending={goals.isPending || tasks.isPending}
        error={goals.error ?? tasks.error}
        onRetry={() => {
          goals.refetch();
          tasks.refetch();
        }}
        isRetrying={goals.isFetching || tasks.isFetching}
      >
        <Section
          title={m.evolution_goals()}
          description={m.evolution_goals_desc()}
          actions={
            <Button size="sm" variant="outline" onClick={() => setGoalDraft(emptyGoal())}>
              <Plus aria-hidden className="size-3.5" />
              {m.evolution_add_goal()}
            </Button>
          }
        >
          {goalDraft && (
            <Card padding="md" className="mb-3 border-primary/30">
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
                <div className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.evolution_goal_title()}</span>
                  <Input
                    value={goalDraft.title}
                    onChange={(event) => setGoalDraft({ ...goalDraft, title: event.target.value })}
                    placeholder={m.evolution_goal_title()}
                  />
                </div>
                <div className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.evolution_goal_kind()}</span>
                  <Select
                    value={goalDraft.kind}
                    onValueChange={(kind) => setGoalDraft({ ...goalDraft, kind })}
                    options={GOAL_KINDS.map((item) => ({ value: item.value, label: item.label() }))}
                    aria-label={m.evolution_goal_kind()}
                  />
                </div>
                <div className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.evolution_goal_period()}</span>
                  <Select
                    value={goalDraft.period}
                    onValueChange={(period) => setGoalDraft({ ...goalDraft, period })}
                    options={GOAL_PERIODS.map((item) => ({
                      value: item.value,
                      label: item.label(),
                    }))}
                    aria-label={m.evolution_goal_period()}
                  />
                </div>
                <div className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.evolution_goal_target()}</span>
                  <NumberField
                    value={goalDraft.target_value}
                    onCommit={(target_value) => setGoalDraft({ ...goalDraft, target_value })}
                    min={1}
                    max={100000}
                    aria-label={m.evolution_goal_target()}
                  />
                </div>
                <div className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.evolution_goal_list()}</span>
                  <Select
                    value={goalDraft.block_list_id ?? ""}
                    onValueChange={(value) =>
                      setGoalDraft({ ...goalDraft, block_list_id: value || null })
                    }
                    options={listOptions}
                    aria-label={m.evolution_goal_list()}
                  />
                </div>
              </div>
              <div className="mt-3 flex gap-2">
                <Button
                  size="sm"
                  disabled={
                    !goalDraft.title.trim() || goalDraft.target_value < 1 || saveGoal.isPending
                  }
                  onClick={() =>
                    saveGoal.mutate(
                      { ...goalDraft, updated_at: new Date().toISOString() },
                      { onSuccess: () => setGoalDraft(null) },
                    )
                  }
                >
                  {m.common_save()}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setGoalDraft(null)}>
                  {m.common_cancel()}
                </Button>
              </div>
            </Card>
          )}

          {(goals.data ?? []).length === 0 ? (
            <EmptyState icon={<Flag />} title={m.evolution_no_goals()} />
          ) : (
            <div className="grid gap-3 sm:grid-cols-2">
              {(goals.data ?? []).map((goal) => {
                const measured = progressById.get(goal.id);
                const units =
                  goal.kind === "time"
                    ? formatDuration(measured?.current ?? 0)
                    : String(measured?.current ?? 0);
                return (
                  <Card key={goal.id} padding="md" elevation="raised">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="truncate font-medium text-foreground">{goal.title}</p>
                        <p className="mt-1 flex flex-wrap items-center gap-1.5 text-muted-foreground text-xs">
                          <Badge tone="neutral">
                            {GOAL_KINDS.find((kind) => kind.value === goal.kind)?.label()}
                          </Badge>
                          <span>{GOAL_PERIODS.find((p) => p.value === goal.period)?.label()}</span>
                          {goal.state !== "active" && (
                            <Badge tone="success">{goalStateLabel(goal.state)}</Badge>
                          )}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        <Button size="sm" variant="ghost" onClick={() => setGoalDraft({ ...goal })}>
                          {m.common_edit()}
                        </Button>
                        <ConfirmButton
                          size="sm"
                          variant="ghost"
                          onConfirm={() => deleteGoal.mutate(goal.id)}
                          disabled={deleteGoal.isPending}
                        >
                          {m.evolution_delete()}
                        </ConfirmButton>
                      </div>
                    </div>

                    <div className="mt-3">
                      <div className="mb-1 flex items-center justify-between text-xs">
                        <span className="text-faint-foreground">{m.evolution_goal_progress()}</span>
                        <span className="text-foreground tabular-nums">
                          {m.evolution_goal_current({
                            current: units,
                            target:
                              goal.kind === "time"
                                ? formatDuration(goal.target_value * 60)
                                : goal.target_value,
                          })}
                        </span>
                      </div>
                      <Progress
                        value={(measured?.percent ?? 0) / 100}
                        label={m.evolution_goal_progress()}
                        tone={measured?.reached ? "success" : "default"}
                      />
                    </div>

                    {goal.kind === "result" && measured?.reached && goal.state === "active" && (
                      <Button
                        size="sm"
                        variant="soft"
                        className="mt-3"
                        disabled={saveGoal.isPending}
                        onClick={() =>
                          saveGoal.mutate({
                            ...goal,
                            state: "completed",
                            completed_at: new Date().toISOString(),
                            updated_at: new Date().toISOString(),
                          })
                        }
                      >
                        <CheckCircle2 aria-hidden className="size-3.5" />
                        {m.evolution_task_complete()}
                      </Button>
                    )}
                  </Card>
                );
              })}
            </div>
          )}
        </Section>

        <Section title={m.evolution_tasks()} description={m.evolution_tasks_desc()}>
          <Card padding="md" className="mb-3">
            <div className="flex flex-wrap items-end gap-2">
              <Input
                value={taskTitle}
                onChange={(event) => setTaskTitle(event.target.value)}
                placeholder={m.evolution_task_title()}
                aria-label={m.evolution_task_title()}
                className="min-w-48 flex-1"
              />
              <Select
                value={taskGoal}
                onValueChange={setTaskGoal}
                options={[
                  { value: "", label: m.evolution_no_goals_option() },
                  ...(goals.data ?? []).map((goal) => ({ value: goal.id, label: goal.title })),
                ]}
                size="sm"
                aria-label={m.evolution_task_goal()}
              />
              <NumberField
                value={taskEstimate}
                onCommit={setTaskEstimate}
                min={1}
                max={1440}
                suffix={m.common_minutes()}
                aria-label={m.evolution_task_estimate()}
              />
              <Button
                size="sm"
                disabled={!taskTitle.trim() || saveTask.isPending}
                onClick={() => {
                  const task = emptyTask();
                  saveTask.mutate(
                    {
                      ...task,
                      title: taskTitle.trim(),
                      goal_id: taskGoal || null,
                      estimate_minutes: taskEstimate,
                    },
                    { onSuccess: () => setTaskTitle("") },
                  );
                }}
              >
                <Plus aria-hidden className="size-3.5" />
                {m.common_add()}
              </Button>
            </div>
          </Card>

          {(tasks.data ?? []).length === 0 ? (
            <EmptyState icon={<ListTodo />} title={m.evolution_no_tasks()} />
          ) : (
            <div className="flex flex-col gap-2">
              {(tasks.data ?? []).map((task) => {
                const done = task.status === "done";
                return (
                  <Card key={task.id} padding="sm">
                    <div className="flex items-center gap-3">
                      <button
                        type="button"
                        aria-label={done ? m.evolution_task_reopen() : m.evolution_task_complete()}
                        aria-pressed={done}
                        className="text-faint-foreground hover:text-success"
                        onClick={() =>
                          saveTask.mutate({
                            ...task,
                            status: done ? "todo" : "done",
                            completed_at: done ? null : new Date().toISOString(),
                            updated_at: new Date().toISOString(),
                          })
                        }
                      >
                        {done ? (
                          <CheckCircle2 aria-hidden className="size-4 text-success" />
                        ) : (
                          <Circle aria-hidden className="size-4" />
                        )}
                      </button>
                      <span
                        className={
                          done
                            ? "min-w-0 flex-1 text-muted-foreground text-sm line-through"
                            : "min-w-0 flex-1 text-foreground text-sm"
                        }
                      >
                        {task.title}
                      </span>
                      {task.estimate_minutes && (
                        <span className="text-faint-foreground text-xs tabular-nums">
                          {task.estimate_minutes} {m.common_minutes()}
                        </span>
                      )}
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-expanded={
                          (task.checklist ?? []).length > 0 || checklistTask === task.id
                        }
                        onClick={() => {
                          setChecklistTask(checklistTask === task.id ? null : task.id);
                          setChecklistText("");
                        }}
                      >
                        <ListChecks aria-hidden className="size-3.5" />
                        {(task.checklist ?? []).length}
                      </Button>
                      <Select
                        value={task.status ?? "todo"}
                        onValueChange={(status) => setTaskStatus(task, status)}
                        options={TASK_STATUSES.map((item) => ({
                          value: item.value,
                          label: item.label(),
                        }))}
                        size="sm"
                        aria-label={m.evolution_task_status()}
                      />
                      <ConfirmButton
                        size="sm"
                        variant="ghost"
                        onConfirm={() => deleteTask.mutate(task.id)}
                        disabled={deleteTask.isPending}
                      >
                        {m.evolution_delete()}
                      </ConfirmButton>
                    </div>

                    {((task.checklist ?? []).length > 0 || checklistTask === task.id) && (
                      <div className="mt-2 flex flex-col gap-1.5 border-border/60 border-t pt-2">
                        {(task.checklist ?? []).map((item) => (
                          <div key={item.id} className="flex items-center gap-2 text-xs">
                            <button
                              type="button"
                              aria-pressed={item.done}
                              aria-label={item.text}
                              className="text-faint-foreground hover:text-success"
                              onClick={() =>
                                saveTask.mutate({
                                  ...task,
                                  checklist: (task.checklist ?? []).map((entry) =>
                                    entry.id === item.id ? { ...entry, done: !entry.done } : entry,
                                  ),
                                  updated_at: new Date().toISOString(),
                                })
                              }
                            >
                              {item.done ? (
                                <CheckCircle2 aria-hidden className="size-3.5 text-success" />
                              ) : (
                                <Circle aria-hidden className="size-3.5" />
                              )}
                            </button>
                            <span
                              className={
                                item.done
                                  ? "flex-1 text-muted-foreground line-through"
                                  : "flex-1 text-foreground"
                              }
                            >
                              {item.text}
                            </span>
                            <Button
                              size="sm"
                              variant="ghost"
                              aria-label={m.common_remove()}
                              disabled={saveTask.isPending}
                              onClick={() =>
                                saveTask.mutate({
                                  ...task,
                                  checklist: (task.checklist ?? []).filter(
                                    (entry) => entry.id !== item.id,
                                  ),
                                  updated_at: new Date().toISOString(),
                                })
                              }
                            >
                              ×
                            </Button>
                          </div>
                        ))}
                        {checklistTask === task.id && (
                          <form
                            className="flex items-center gap-2"
                            onSubmit={(event) => {
                              event.preventDefault();
                              addChecklistItem(task);
                            }}
                          >
                            <Input
                              value={checklistText}
                              onChange={(event) => setChecklistText(event.target.value)}
                              placeholder={m.evolution_checklist_placeholder()}
                              aria-label={m.evolution_checklist_placeholder()}
                              className="h-8 flex-1 text-xs"
                            />
                            <Button
                              type="submit"
                              size="sm"
                              variant="soft"
                              disabled={!checklistText.trim() || saveTask.isPending}
                            >
                              {m.evolution_checklist_add()}
                            </Button>
                          </form>
                        )}
                      </div>
                    )}
                  </Card>
                );
              })}
            </div>
          )}
        </Section>

        <div className="mt-7 grid gap-5 lg:grid-cols-2">
          <section>
            <h2 className="mb-1 font-medium text-foreground text-sm">{m.evolution_focus()}</h2>
            <p className="mb-3 text-muted-foreground text-xs">{m.evolution_focus_desc()}</p>
            <Card padding="md" elevation="raised">
              {activeSession ? (
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <p className="font-medium text-foreground">
                      {activeSession.label || m.evolution_focus()}
                    </p>
                    <p className="mt-0.5 text-muted-foreground text-xs">
                      {m.evolution_focus_running({
                        when: new Date(activeSession.started_at).toLocaleTimeString(undefined, {
                          hour: "2-digit",
                          minute: "2-digit",
                        }),
                      })}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="soft"
                    disabled={finishFocus.isPending}
                    onClick={() => finishFocus.mutate(activeSession.id)}
                  >
                    {m.evolution_focus_finish()}
                  </Button>
                </div>
              ) : (
                <div className="flex flex-wrap items-end gap-2">
                  <Input
                    value={focusLabel}
                    onChange={(event) => setFocusLabel(event.target.value)}
                    placeholder={m.evolution_focus_label()}
                    aria-label={m.evolution_focus_label()}
                    className="min-w-48 flex-1"
                  />
                  <Select
                    value={focusGoal}
                    onValueChange={setFocusGoal}
                    options={[
                      { value: "", label: m.evolution_no_goals_option() },
                      ...(goals.data ?? []).map((goal) => ({ value: goal.id, label: goal.title })),
                    ]}
                    size="sm"
                    aria-label={m.evolution_task_goal()}
                  />
                  <Button
                    size="sm"
                    disabled={startFocus.isPending}
                    onClick={() =>
                      startFocus.mutate(
                        { label: focusLabel.trim(), goalId: focusGoal || null, taskId: null },
                        { onSuccess: () => setFocusLabel("") },
                      )
                    }
                  >
                    <Timer aria-hidden className="size-3.5" />
                    {m.evolution_focus_start()}
                  </Button>
                </div>
              )}
            </Card>

            <h3 className="mt-4 mb-2 font-medium text-foreground text-xs">
              {m.evolution_focus_history()}
            </h3>
            {(sessions.data ?? []).length === 0 ? (
              <p className="text-faint-foreground text-xs">{m.evolution_no_sessions()}</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {(sessions.data ?? [])
                  .filter((session) => session.ended_at)
                  .slice(0, 8)
                  .map((session) => (
                    <div
                      key={session.id}
                      className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface/50 px-3 py-2 text-xs"
                    >
                      <span className="truncate text-foreground">
                        {session.label || m.evolution_focus()}
                      </span>
                      <span className="shrink-0 text-muted-foreground tabular-nums">
                        {m.evolution_focus_minutes({
                          minutes: Math.round(session.focused_secs / 60),
                        })}
                      </span>
                    </div>
                  ))}
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-1 flex items-center gap-1.5 font-medium text-foreground text-sm">
              <Sparkles aria-hidden className="size-4 text-primary" />
              {m.evolution_coach()}
            </h2>
            <p className="mb-3 text-muted-foreground text-xs">{m.evolution_coach_desc()}</p>
            {coach.error ? (
              <InlineError error={coach.error} />
            ) : (coach.data ?? []).length === 0 ? (
              <Card padding="md" className="text-muted-foreground text-sm">
                {m.evolution_no_insights()}
              </Card>
            ) : (
              <div className="flex flex-col gap-2">
                {(coach.data ?? []).map((insight) => (
                  <Card key={insight.id} padding="md" className="border-primary/20">
                    <div className="flex items-start justify-between gap-2">
                      <p className="font-medium text-foreground text-sm">{insight.title}</p>
                      <Badge
                        tone={
                          insight.severity === "high"
                            ? "destructive"
                            : insight.severity === "normal"
                              ? "warning"
                              : "neutral"
                        }
                      >
                        {insight.severity}
                      </Badge>
                    </div>
                    <p className="mt-1 text-muted-foreground text-xs leading-relaxed">
                      {insight.body}
                    </p>
                    <p className="mt-2 text-faint-foreground text-[11px]">
                      {m.evolution_coach_evidence()}: {insight.evidence.join(" · ")}
                    </p>
                  </Card>
                ))}
              </div>
            )}
          </section>
        </div>

        <Section
          title={m.notifications_title()}
          actions={
            (notifications.data ?? []).length > 0 ? (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => markAll.mutate()}
                disabled={markAll.isPending}
              >
                {m.notifications_mark_all_read()}
              </Button>
            ) : undefined
          }
        >
          {(notifications.data ?? []).length === 0 ? (
            <p className="text-faint-foreground text-xs">{m.notifications_empty()}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {(notifications.data ?? []).slice(0, 10).map((notification) => (
                <Card key={notification.id} padding="sm" className="flex items-start gap-3">
                  <Badge tone={notification.priority === "high" ? "destructive" : "neutral"}>
                    {notificationKindLabel(notification.kind)}
                  </Badge>
                  <div className="min-w-0">
                    <p className="font-medium text-foreground text-sm">{notification.title}</p>
                    <p className="text-muted-foreground text-xs">{notification.body}</p>
                  </div>
                </Card>
              ))}
            </div>
          )}
        </Section>

        <InlineError
          error={
            saveGoal.error ??
            deleteGoal.error ??
            saveTask.error ??
            deleteTask.error ??
            startFocus.error ??
            finishFocus.error
          }
        />
      </QueryState>
    </Page>
  );
}
