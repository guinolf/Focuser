import { Lock, ShieldCheck, Unlock } from "lucide-react";
import { useEffect, useState } from "react";
import type { ProtectionPolicy } from "@/bindings";
import {
  effectiveLock,
  ProtectDialog,
  protectionActive,
  UnlockDialog,
} from "@/components/focus-lock-forms";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, EmptyState, PageHeader } from "@/components/ui/card";
import { InlineError, QueryState } from "@/components/ui/feedback";
import { NumberField } from "@/components/ui/number-field";
import { Page } from "@/components/ui/page";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  useBlockLists,
  useProtectionPolicy,
  useSetProtectionPolicy,
  useUnlockStatus,
} from "@/lib/commands";
import { formatCountdown, formatDuration } from "@/lib/duration";
import { CHALLENGE_LENGTH, MIN_WAIT_SECS, PRESETS, presetMeta } from "@/lib/protection";
import { m } from "@/paraglide/messages.js";

export function Protection() {
  const lists = useBlockLists();
  const [selectedId, setSelectedId] = useState<string>("");
  const [protecting, setProtecting] = useState(false);
  const [unlocking, setUnlocking] = useState(false);

  const all = lists.data ?? [];
  const selected = all.find((list) => list.id === selectedId) ?? all[0] ?? null;
  const policy = useProtectionPolicy(selected?.id ?? null);
  const status = useUnlockStatus(selected?.id ?? null);
  const [draft, setDraft] = useState<ProtectionPolicy | null>(null);
  const save = useSetProtectionPolicy();

  useEffect(() => {
    if (policy.data) setDraft(policy.data);
  }, [policy.data]);

  const locked = selected ? protectionActive(selected.protection) : false;
  const scheduled = selected ? !!selected.scheduled_protection : false;
  const canUnlock = selected ? effectiveLock(selected) !== null : false;

  return (
    <Page>
      <PageHeader title={m.protection_title()} description={m.protection_description()} />

      <QueryState
        isPending={lists.isPending}
        error={lists.error}
        onRetry={() => lists.refetch()}
        isRetrying={lists.isFetching}
      >
        {all.length === 0 ? (
          <EmptyState icon={<ShieldCheck />} title={m.protection_no_protected()} />
        ) : (
          <>
            <Card padding="md" className="mb-4">
              <div className="flex flex-wrap items-end gap-3">
                <div className="flex min-w-56 flex-1 flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{m.protection_list_title()}</span>
                  <Select
                    value={selected?.id ?? ""}
                    onValueChange={setSelectedId}
                    options={all.map((list) => ({ value: list.id, label: list.name }))}
                    aria-label={m.protection_list_title()}
                  />
                </div>
                {selected && (
                  <div className="flex flex-wrap items-center gap-2">
                    {locked && <Badge tone="warning">{m.protection_locked()}</Badge>}
                    {scheduled && <Badge tone="neutral">{m.protection_scheduled()}</Badge>}
                    {canUnlock ? (
                      <Button size="sm" variant="outline" onClick={() => setUnlocking(true)}>
                        <Unlock aria-hidden className="size-3.5" />
                        {m.protection_unlock_action()}
                      </Button>
                    ) : (
                      <Button size="sm" onClick={() => setProtecting(true)} disabled={locked}>
                        <Lock aria-hidden className="size-3.5" />
                        {m.protection_lock_action()}
                      </Button>
                    )}
                  </div>
                )}
              </div>
            </Card>

            {selected && draft && (
              <div className="grid gap-4 lg:grid-cols-2">
                <Card padding="md" elevation="raised">
                  <h2 className="font-medium text-foreground text-sm">{m.protection_advanced()}</h2>
                  <p className="mt-1 text-muted-foreground text-xs">
                    {locked ? m.protection_locked_hint() : m.protection_unprotected_hint()}
                  </p>

                  <fieldset className="mt-4">
                    <legend className="text-muted-foreground text-xs">
                      {m.protection_preset()}
                    </legend>
                    <div className="mt-1.5 flex flex-wrap gap-1.5">
                      {PRESETS.map((preset) => (
                        <Button
                          key={preset.id}
                          type="button"
                          size="sm"
                          variant={draft.preset === preset.id ? "soft" : "outline"}
                          aria-pressed={draft.preset === preset.id}
                          onClick={() => {
                            const meta = presetMeta(preset.id);
                            setDraft({
                              ...draft,
                              preset: preset.id,
                              prevent_uninstall: meta.uninstall,
                              prevent_service_stop: meta.serviceStop,
                              prevent_modification: meta.modification,
                              wait_secs: Math.max(MIN_WAIT_SECS, meta.waitSecs),
                            });
                          }}
                        >
                          {preset.label()}
                        </Button>
                      ))}
                    </div>
                    <p className="mt-1.5 text-faint-foreground text-xs">
                      {presetMeta(draft.preset).hint()}
                    </p>
                  </fieldset>

                  <div className="mt-4 grid gap-4 sm:grid-cols-2">
                    <div>
                      <p className="text-muted-foreground text-xs">{m.protection_wait()}</p>
                      <NumberField
                        value={Math.round(draft.wait_secs / 60)}
                        onCommit={(minutes) =>
                          setDraft({ ...draft, wait_secs: Math.max(MIN_WAIT_SECS, minutes * 60) })
                        }
                        min={10}
                        max={1440}
                        step={5}
                        suffix={m.common_minutes()}
                        aria-label={m.protection_wait()}
                      />
                      <p className="mt-1 text-faint-foreground text-[11px]">
                        {m.protection_wait_hint()}
                      </p>
                    </div>
                    <div>
                      <p className="text-muted-foreground text-xs">{m.protection_attempts()}</p>
                      <NumberField
                        value={draft.max_attempts_per_hour}
                        onCommit={(attempts) =>
                          setDraft({ ...draft, max_attempts_per_hour: attempts })
                        }
                        min={1}
                        max={20}
                        aria-label={m.protection_attempts()}
                      />
                    </div>
                  </div>

                  <div className="mt-4">
                    <p className="text-muted-foreground text-xs">{m.protection_guards()}</p>
                    <div className="mt-2 flex flex-col gap-2">
                      {[
                        { key: "prevent_uninstall", label: m.lists_guard_uninstall() },
                        { key: "prevent_service_stop", label: m.lists_guard_service() },
                        { key: "prevent_modification", label: m.lists_guard_edit() },
                      ].map(({ key, label }) => (
                        <div key={key} className="flex items-center gap-2 text-sm">
                          <Switch
                            size="sm"
                            checked={draft[key as "prevent_uninstall"]}
                            onCheckedChange={(checked) => setDraft({ ...draft, [key]: checked })}
                            aria-label={label}
                          />
                          <span className="text-muted-foreground">{label}</span>
                        </div>
                      ))}
                    </div>
                  </div>

                  <div className="mt-4 flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">{m.protection_challenge_len()}</span>
                    <Badge tone="neutral">
                      {CHALLENGE_LENGTH} · {m.protection_challenge_length_fixed()}
                    </Badge>
                  </div>

                  <div className="mt-4 flex items-center gap-2">
                    <Button
                      size="sm"
                      disabled={save.isPending}
                      onClick={() => save.mutate({ listId: selected.id, policy: draft })}
                    >
                      {m.protection_save()}
                    </Button>
                    {save.isSuccess && (
                      <span className="text-success text-xs">{m.protection_saved()}</span>
                    )}
                  </div>
                  <InlineError error={save.error} />
                </Card>

                <div className="flex flex-col gap-4">
                  <Card padding="md" elevation="raised">
                    <h2 className="font-medium text-foreground text-sm">
                      {m.protection_how_it_works()}
                    </h2>
                    <p className="mt-1 text-muted-foreground text-xs leading-relaxed">
                      {m.protection_restart_note()}
                    </p>
                    {status.data && (
                      <dl className="mt-4 grid grid-cols-2 gap-3 text-xs">
                        <div>
                          <dt className="text-faint-foreground">{m.protection_wait()}</dt>
                          <dd className="mt-0.5 font-medium text-foreground">
                            {formatDuration(draft.wait_secs)}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-faint-foreground">{m.protection_attempts()}</dt>
                          <dd className="mt-0.5 font-medium text-foreground">
                            {status.data.attempts_remaining}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-faint-foreground">
                            {m.protection_status_waiting({ time: "" })}
                          </dt>
                          <dd className="mt-0.5 font-medium text-foreground">
                            {status.data.state === "waiting"
                              ? formatCountdown(status.data.remaining_secs)
                              : "—"}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-faint-foreground">{m.protection_challenge_len()}</dt>
                          <dd className="mt-0.5 font-medium text-foreground">
                            {status.data.challenge_len}
                          </dd>
                        </div>
                      </dl>
                    )}
                    <InlineError error={status.error} />
                  </Card>

                  <Card padding="md">
                    <p className="text-muted-foreground text-xs">{m.protection_no_protected()}</p>
                  </Card>
                </div>
              </div>
            )}
          </>
        )}
      </QueryState>

      {selected && (
        <>
          <ProtectDialog list={selected} open={protecting} onClose={() => setProtecting(false)} />
          <UnlockDialog list={selected} open={unlocking} onClose={() => setUnlocking(false)} />
        </>
      )}
    </Page>
  );
}
