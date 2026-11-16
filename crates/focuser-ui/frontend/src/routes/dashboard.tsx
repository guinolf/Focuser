import { Activity, Ban, Clock, ListChecks, ListPlus, Lock, ShieldCheck } from "lucide-react";
import { Link } from "react-router-dom";
import { FocusSession } from "@/components/focus-session";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, PageHeader, Section } from "@/components/ui/card";
import { InlineError } from "@/components/ui/feedback";
import { Page } from "@/components/ui/page";
import { StatGridSkeleton } from "@/components/ui/skeleton";
import { Stat, StatGrid } from "@/components/ui/stat";
import { Switch } from "@/components/ui/switch";
import { Tooltip } from "@/components/ui/tooltip";
import {
  useActivityOverview,
  useBlockLists,
  usePlanRuntimes,
  useProtectionStatus,
  useStats,
  useToggleBlockList,
} from "@/lib/commands";
import { rangeFor } from "@/lib/date-range";
import { formatDuration } from "@/lib/duration";
import { planStatusLabel, planStatusTone } from "@/lib/plan-labels";
import { m } from "@/paraglide/messages.js";

export function Dashboard() {
  const lists = useBlockLists();
  const protection = useProtectionStatus();
  const toggle = useToggleBlockList();

  const today = rangeFor("today");
  const stats = useStats(today.from, today.to);
  const activity = useActivityOverview(today.from, today.to);
  const plans = usePlanRuntimes();

  const all = lists.data ?? [];
  const enabled = all.filter((l) => l.enabled);
  const rules = enabled.reduce((n, l) => n + l.websites.length + l.applications.length, 0);
  const blockedToday = (stats.data ?? []).reduce((n, s) => n + s.blocked_attempts, 0);
  const locked = protection.data ?? [];
  // A dash is not a zero: disabled, unavailable and not-yet-collected activity
  // must never look like a measured day of no use.
  const activityMeasurement = activity.data?.measurement;
  const activityIsNumbers =
    activityMeasurement === "recorded" || activityMeasurement === "nothing_in_range";
  // Locks with no end never unlock on their own, so they have no "soonest".
  const soonest = locked.reduce<number | null>(
    (min, p) =>
      p.remaining_seconds !== null && (min === null || p.remaining_seconds < min)
        ? p.remaining_seconds
        : min,
    null,
  );

  return (
    <Page wide>
      <PageHeader title={m.dashboard_title()} description={m.dashboard_description()} />

      {lists.isPending ? (
        <StatGridSkeleton />
      ) : all.length === 0 ? (
        <Card padding="md" elevation="raised">
          <Badge tone="info">{m.onboarding_badge()}</Badge>
          <h2 className="mt-2 font-semibold text-foreground text-lg">{m.onboarding_title()}</h2>
          <p className="mt-1 max-w-2xl text-muted-foreground text-sm">
            {m.onboarding_description()}
          </p>
          <ol className="mt-4 grid gap-3 text-sm sm:grid-cols-3">
            {[m.onboarding_step_list(), m.onboarding_step_schedule(), m.onboarding_step_lock()].map(
              (step, index) => (
                <li key={step} className="flex gap-2.5">
                  <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary/10 font-medium text-primary text-xs">
                    {index + 1}
                  </span>
                  <span className="text-muted-foreground">{step}</span>
                </li>
              ),
            )}
          </ol>
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <Button asChild>
              <Link to="/block-lists">
                <ListPlus aria-hidden className="size-4" />
                {m.dashboard_create_list()}
              </Link>
            </Button>
            <span className="text-faint-foreground text-xs">{m.onboarding_privacy()}</span>
          </div>
        </Card>
      ) : (
        <StatGrid>
          <Stat
            label={m.dashboard_blocked_today()}
            value={blockedToday.toLocaleString()}
            icon={<Ban aria-hidden />}
            tone="primary"
            hint={
              blockedToday === 0
                ? m.dashboard_blocked_today_none()
                : m.dashboard_blocked_today_hint()
            }
          />
          <Stat
            label={m.dashboard_lists_enabled()}
            value={m.dashboard_lists_enabled_value({ enabled: enabled.length, total: all.length })}
            icon={<ListChecks aria-hidden />}
            footer={<EnabledBar enabled={enabled.length} total={all.length} />}
          />
          <Stat
            label={m.dashboard_rules_in_force()}
            value={rules.toLocaleString()}
            icon={<ShieldCheck aria-hidden />}
            hint={m.dashboard_rules_in_force_hint()}
          />
          <Stat
            label={m.dashboard_protected()}
            value={locked.length}
            icon={<Lock aria-hidden />}
            tone={locked.length > 0 ? "warning" : "default"}
            hint={
              soonest !== null
                ? m.dashboard_protected_unlocks({ duration: formatDuration(soonest) })
                : locked.length > 0
                  ? m.dashboard_protected_until_unlocked()
                  : m.dashboard_protected_none()
            }
          />
        </StatGrid>
      )}

      <Card padding="md" elevation="raised" className="mt-4">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="font-medium text-foreground text-sm">{m.dashboard_today()}</p>
            <p className="mt-0.5 text-muted-foreground text-xs">
              {activityIsNumbers
                ? m.dashboard_today_hint()
                : activityMeasurement === "disabled"
                  ? m.activity_measurement_disabled()
                  : activityMeasurement === "unavailable"
                    ? m.activity_measurement_unavailable()
                    : m.activity_measurement_awaiting()}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-5 text-sm">
            <span className="flex items-center gap-1.5">
              <Clock aria-hidden className="size-3.5 text-primary" />
              <span className="text-faint-foreground">{m.activity_foreground()}:</span>
              <span className="font-medium tabular-nums">
                {activityIsNumbers ? formatDuration(activity.data?.foreground_secs ?? 0) : "—"}
              </span>
            </span>
            <span className="flex items-center gap-1.5">
              <Activity aria-hidden className="size-3.5 text-success" />
              <span className="text-faint-foreground">{m.activity_interaction()}:</span>
              <span className="font-medium tabular-nums">
                {activityIsNumbers ? formatDuration(activity.data?.interaction_secs ?? 0) : "—"}
              </span>
            </span>
            <Button asChild size="sm" variant="ghost">
              <Link to="/activity">{m.dashboard_open_activity()}</Link>
            </Button>
          </div>
        </div>

        <div className="mt-4 border-border/60 border-t pt-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="font-medium text-foreground text-xs">{m.dashboard_active_plans()}</p>
            <Button asChild size="sm" variant="ghost">
              <Link to="/plans">{m.dashboard_open_plans()}</Link>
            </Button>
          </div>
          {(plans.data ?? []).filter((plan) => plan.armed).length === 0 ? (
            <p className="mt-1 text-faint-foreground text-xs">{m.dashboard_no_plans()}</p>
          ) : (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {(plans.data ?? [])
                .filter((plan) => plan.armed)
                .slice(0, 6)
                .map((plan) => {
                  const list = all.find((item) => item.id === plan.block_list_id);
                  return (
                    <Badge key={plan.block_list_id} tone={planStatusTone(plan.status)}>
                      {list?.name ?? plan.block_list_id} · {planStatusLabel(plan.status)}
                    </Badge>
                  );
                })}
            </div>
          )}
        </div>
      </Card>

      {all.length > 0 && (
        <>
          <div className="mt-7">
            <FocusSession lists={all} />
          </div>

          <Section
            title={m.dashboard_lists_title()}
            description={m.dashboard_lists_description()}
            actions={
              <Button asChild variant="outline" size="sm">
                <Link to="/block-lists">
                  <ListPlus aria-hidden className="size-4" />
                  {m.dashboard_manage()}
                </Link>
              </Button>
            }
          >
            <div className="flex flex-col gap-2">
              {all.map((list) => {
                const lock = locked.find((p) => p.block_list_id === list.id);
                return (
                  <Card
                    key={list.id}
                    padding="none"
                    elevation="raised"
                    className="flex items-center justify-between gap-4 px-4 py-3.5 transition-colors hover:border-border-strong"
                  >
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <p className="truncate font-medium text-foreground text-sm">{list.name}</p>
                        {lock && (
                          <Badge tone="warning" icon={<Lock aria-hidden />}>
                            {lock.remaining_seconds === null
                              ? m.dashboard_protected_until_unlocked()
                              : formatDuration(lock.remaining_seconds)}
                          </Badge>
                        )}
                        {list.schedule && <Badge tone="info">{m.dashboard_scheduled()}</Badge>}
                      </div>
                      <p className="mt-0.5 text-faint-foreground text-xs">
                        {m.count_sites({ count: list.websites.length })} ·{" "}
                        {m.count_apps({ count: list.applications.length })}
                      </p>
                    </div>

                    {lock ? (
                      <Tooltip content={m.dashboard_locked_tooltip()}>
                        <span>
                          <Switch
                            checked={list.enabled}
                            onCheckedChange={() => {}}
                            disabled
                            aria-label={m.dashboard_enable_list({ name: list.name })}
                          />
                        </span>
                      </Tooltip>
                    ) : (
                      <Switch
                        checked={list.enabled}
                        onCheckedChange={(next) => toggle.mutate({ id: list.id, enabled: next })}
                        disabled={toggle.isPending && toggle.variables?.id === list.id}
                        aria-label={m.dashboard_enable_list({ name: list.name })}
                      />
                    )}
                  </Card>
                );
              })}
            </div>
            <InlineError error={lists.error ?? toggle.error} />
          </Section>
        </>
      )}
    </Page>
  );
}

/** How much of the collection is switched on, at a glance. */
function EnabledBar({ enabled, total }: { enabled: number; total: number }) {
  if (total === 0) return null;

  return (
    <div className="flex gap-1" aria-hidden>
      {Array.from({ length: total }, (_, i) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: positions, not entities
          key={i}
          className={`h-1 flex-1 rounded-full ${i < enabled ? "bg-primary" : "bg-elevated"}`}
        />
      ))}
    </div>
  );
}
