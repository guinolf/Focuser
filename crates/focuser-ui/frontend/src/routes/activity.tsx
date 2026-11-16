import { Activity, AppWindow, Clock, Globe, Moon, MousePointerClick } from "lucide-react";
import { useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, EmptyState, PageHeader, Section } from "@/components/ui/card";
import { ConfirmButton } from "@/components/ui/confirm-button";
import { InlineError, QueryState } from "@/components/ui/feedback";
import { Page } from "@/components/ui/page";
import { Select } from "@/components/ui/select";
import { Stat, StatGrid } from "@/components/ui/stat";
import { useActivityHealth, useActivityOverview, useClearActivity } from "@/lib/commands";
import { formatDay, type RangeId, rangeFor } from "@/lib/date-range";
import { formatDuration } from "@/lib/duration";
import { m } from "@/paraglide/messages.js";

const RANGE_OPTIONS = [
  { value: "today", label: () => m.activity_range_today() },
  { value: "7d", label: () => m.activity_range_7() },
  { value: "30d", label: () => m.activity_range_30() },
] as const;

function measurementText(measurement: string): string {
  switch (measurement) {
    case "unavailable":
      return m.activity_measurement_unavailable();
    case "disabled":
      return m.activity_measurement_disabled();
    case "awaiting_data":
      return m.activity_measurement_awaiting();
    case "nothing_in_range":
      return m.activity_measurement_nothing();
    default:
      return m.activity_measurement_recorded();
  }
}

function formatWhen(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ActivityPage() {
  const [rangeId, setRangeId] = useState<RangeId>("today");
  const range = useMemo(() => rangeFor(rangeId), [rangeId]);
  const overview = useActivityOverview(range.from, range.to);
  const health = useActivityHealth();
  const clear = useClearActivity();

  const data = overview.data;
  const days = data?.days ?? [];
  const targets = useMemo(
    () => [...(data?.targets ?? [])].sort((a, b) => b.foreground_secs - a.foreground_secs),
    [data?.targets],
  );
  const busiest = days.reduce((max, day) => Math.max(max, day.foreground_secs), 1);

  return (
    <Page wide>
      <PageHeader
        title={m.activity_title()}
        description={m.activity_description()}
        actions={
          <>
            <Select
              value={rangeId}
              onValueChange={setRangeId}
              options={RANGE_OPTIONS.map((option) => ({
                value: option.value,
                label: option.label(),
              }))}
              size="sm"
              aria-label={m.activity_title()}
            />
            <ConfirmButton
              variant="outline"
              size="sm"
              onConfirm={() => clear.mutate()}
              disabled={clear.isPending || (data?.total_sample_rows ?? 0) === 0}
            >
              {m.activity_clear()}
            </ConfirmButton>
          </>
        }
      />

      <QueryState
        isPending={overview.isPending}
        error={overview.error}
        onRetry={() => overview.refetch()}
        isRetrying={overview.isFetching}
      >
        {data && (
          <>
            <Card
              className={
                data.measurement === "recorded"
                  ? "text-muted-foreground text-sm"
                  : "border-warning/40 bg-warning/5 text-foreground text-sm"
              }
              padding="sm"
            >
              {measurementText(data.measurement)}
            </Card>

            <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
              <Badge tone={data.extension_connected ? "success" : "neutral"}>
                {data.extension_connected
                  ? m.activity_health_extension_connected()
                  : m.activity_health_extension_missing()}
              </Badge>
              <Badge tone={data.app_usage_measurable ? "success" : "neutral"}>
                {data.app_usage_measurable
                  ? m.activity_health_app_available()
                  : m.activity_health_app_unavailable()}
              </Badge>
              <span className="text-faint-foreground">
                {data.last_sample_at
                  ? m.activity_health_last({ when: formatWhen(data.last_sample_at) })
                  : m.activity_health_never()}
              </span>
            </div>

            <StatGrid>
              <Stat
                label={m.activity_foreground()}
                value={formatDuration(data.foreground_secs)}
                hint={m.activity_hint_foreground()}
                icon={<Clock aria-hidden className="size-3.5" />}
                tone="primary"
              />
              <Stat
                label={m.activity_interaction()}
                value={formatDuration(data.interaction_secs)}
                hint={m.activity_hint_interaction()}
                icon={<MousePointerClick aria-hidden className="size-3.5" />}
                tone="success"
              />
              <Stat
                label={m.activity_idle()}
                value={formatDuration(data.idle_secs)}
                icon={<Moon aria-hidden className="size-3.5" />}
              />
              <Stat
                label={m.activity_active_targets()}
                value={data.active_targets}
                icon={<Activity aria-hidden className="size-3.5" />}
              />
            </StatGrid>

            <Section title={m.activity_days()}>
              {days.length === 0 ? (
                <EmptyState
                  icon={<Clock />}
                  title={m.activity_no_targets()}
                  description={measurementText(data.measurement)}
                />
              ) : (
                <Card padding="none" elevation="raised" className="overflow-hidden">
                  <table className="w-full text-sm">
                    <caption className="sr-only">{m.activity_days()}</caption>
                    <thead>
                      <tr className="border-border border-b bg-elevated/40 text-muted-foreground text-xs">
                        <th scope="col" className="px-4 py-2 text-left font-normal">
                          {m.activity_days()}
                        </th>
                        <th scope="col" className="px-4 py-2">
                          <span className="sr-only">{m.activity_foreground()}</span>
                        </th>
                        <th scope="col" className="px-4 py-2 text-right font-normal">
                          {m.activity_foreground()}
                        </th>
                        <th scope="col" className="px-4 py-2 text-right font-normal">
                          {m.activity_interaction()}
                        </th>
                        <th scope="col" className="px-4 py-2 text-right font-normal">
                          {m.activity_idle()}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {days.map((day) => (
                        <tr
                          key={day.date}
                          className="border-border/60 border-b last:border-0 hover:bg-elevated/50"
                        >
                          <td className="px-4 py-2.5 text-foreground">{formatDay(day.date)}</td>
                          <td className="px-4 py-2.5">
                            <div className="ml-auto h-1.5 w-40 overflow-hidden rounded-full bg-elevated">
                              <div
                                className="h-full rounded-full bg-primary/70"
                                style={{
                                  width: `${Math.max(3, (day.foreground_secs / busiest) * 100)}%`,
                                }}
                              />
                            </div>
                          </td>
                          <td className="px-4 py-2.5 text-right tabular-nums">
                            {formatDuration(day.foreground_secs)}
                          </td>
                          <td className="px-4 py-2.5 text-right text-muted-foreground tabular-nums">
                            {formatDuration(day.interaction_secs)}
                          </td>
                          <td className="px-4 py-2.5 text-right text-muted-foreground tabular-nums">
                            {formatDuration(day.idle_secs)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </Card>
              )}
            </Section>

            <Section
              title={m.activity_targets()}
              actions={
                targets.length > 0 ? <Badge tone="neutral">{targets.length}</Badge> : undefined
              }
            >
              {targets.length === 0 ? (
                <EmptyState icon={<Globe />} title={m.activity_no_targets()} />
              ) : (
                <Card padding="none" elevation="raised" className="overflow-hidden">
                  <table className="w-full text-sm">
                    <caption className="sr-only">{m.activity_targets()}</caption>
                    <thead>
                      <tr className="border-border border-b bg-elevated/40 text-muted-foreground text-xs">
                        <th scope="col" className="px-4 py-2 text-left font-normal">
                          {m.activity_targets()}
                        </th>
                        <th scope="col" className="px-4 py-2 text-left font-normal">
                          {m.activity_source_foreground()}
                        </th>
                        <th scope="col" className="px-4 py-2 text-right font-normal">
                          {m.activity_foreground()}
                        </th>
                        <th scope="col" className="px-4 py-2 text-right font-normal">
                          {m.activity_interaction()}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {targets.slice(0, 20).map((target) => (
                        <tr
                          key={`${target.source}-${target.target}`}
                          className="border-border/60 border-b last:border-0"
                        >
                          <td className="px-4 py-2.5 text-foreground">
                            <span className="flex items-center gap-2">
                              {target.source === "browser_tab" ? (
                                <Globe aria-hidden className="size-3.5 text-faint-foreground" />
                              ) : (
                                <AppWindow aria-hidden className="size-3.5 text-faint-foreground" />
                              )}
                              <span className="truncate">{target.target}</span>
                            </span>
                          </td>
                          <td className="px-4 py-2.5 text-muted-foreground">
                            {target.source === "browser_tab"
                              ? m.activity_source_browser()
                              : m.activity_source_foreground()}
                          </td>
                          <td className="px-4 py-2.5 text-right tabular-nums">
                            {formatDuration(target.foreground_secs)}
                          </td>
                          <td className="px-4 py-2.5 text-right text-muted-foreground tabular-nums">
                            {formatDuration(target.interaction_secs)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </Card>
              )}
            </Section>

            <p className="mt-4 text-faint-foreground text-xs">
              {m.activity_retention_hint({ days: health.data?.retention_days ?? 90 })}
            </p>
          </>
        )}
      </QueryState>

      <InlineError error={clear.error} />
    </Page>
  );
}
