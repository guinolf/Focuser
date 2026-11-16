import type { PlanRuntime } from "@/bindings";
import { m } from "@/paraglide/messages.js";

export function planStatusLabel(status: PlanRuntime["status"]): string {
  switch (status) {
    case "active":
      return m.plans_status_active();
    case "paused":
      return m.plans_status_paused();
    case "limit_reached":
      return m.plans_status_limit_reached();
    case "goal_reached":
      return m.plans_status_goal_reached();
    case "monitoring":
      return m.plans_status_monitoring();
    default:
      return m.plans_status_inactive();
  }
}

export function planStatusTone(status: PlanRuntime["status"]) {
  if (status === "active" || status === "limit_reached") return "destructive" as const;
  if (status === "goal_reached") return "success" as const;
  if (status === "paused" || status === "monitoring") return "warning" as const;
  return "neutral" as const;
}
