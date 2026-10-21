// Typed wrappers around the Rust commands. The UI never sees secrets.
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type Person = { name?: string | null; email?: string | null };
export type SourceRef = {
  id: string; source_item_id?: string | null; meeting_id?: string | null; relation: string;
  evidence_quote?: string | null; provider?: string | null; url?: string | null; label?: string | null; added_at: string;
};
export type Task = {
  id: string; title: string; description?: string | null; task_type: string; my_role: string; owner_name?: string | null;
  status: string; priority: number; priority_reason?: string | null; confidence: number; origin: string;
  needs_confirmation: boolean; due_at?: string | null; due_kind: string; due_phrase?: string | null; committed_at?: string | null;
  meeting_id?: string | null; meeting_title?: string | null; project?: string | null; expected_deliverable?: string | null;
  completion_criteria?: string | null; parent_id?: string | null; recurrence?: string | null; effort_minutes?: number | null;
  people: Person[]; counterpart_email?: string | null; possible_duplicate_of?: string | null; conflict_note?: string | null;
  review_reason?: string | null; completion_evidence?: string | null; last_update_at?: string | null; completed_at?: string | null;
  created_at: string; updated_at: string; source_count: number; primary_source?: SourceRef | null; subtask_count: number; subtasks_done: number;
};
export type Deadline = { id: string; kind: string; due_at: string; phrase?: string | null; reason?: string | null; created_at: string; superseded_at?: string | null };
export type AuditEvent = { id: string; action: string; from_value?: string | null; to_value?: string | null; actor: string; reason?: string | null; created_at: string; undone: boolean };
export type TaskDetail = { task: Task; sources: SourceRef[]; deadlines: Deadline[]; history: AuditEvent[]; subtasks: Task[]; depends_on: Task[]; possible_duplicate?: Task | null };
export type Attendee = { email: string; name?: string | null; response?: string | null; optional: boolean };
export type Meeting = {
  id: string; account_id: string; title: string; description?: string | null; location?: string | null; start_at: string; end_at: string;
  timezone?: string | null; all_day: boolean; organizer_email?: string | null; organizer_name?: string | null; join_url?: string | null; url?: string | null;
  status: string; my_response?: string | null; prep_state: string; attendees: Attendee[]; open_task_count: number;
};
export type Commitment = { id: string; source_item_id?: string | null; direction: string; who_name?: string | null; who_email?: string | null; text: string; due_at?: string | null; status: string; task_id?: string | null; created_at: string; source_label?: string | null; source_url?: string | null };
export type RelatedMessage = { source_item_id: string; label: string; subject?: string | null; snippet: string; url?: string | null; sent_at: string };
export type ChecklistItem = { text: string; basis: string; task_id?: string | null; source_item_id?: string | null; done: boolean; due_before_start: boolean };
export type Briefing = {
  meeting: Meeting; duration_minutes: number; local_start: string; has_agenda: boolean; agenda?: string | null; linked_tasks: Task[];
  promises_to_attendees: Task[]; waiting_on_attendees: Commitment[]; previous_followups: Task[]; related_messages: RelatedMessage[];
  open_questions: [string, RelatedMessage][]; checklist: ChecklistItem[]; notes: string[]; ai_summary?: string | null; related_documents: Doc[];
};
export type PlanBlock = { task_id?: string | null; meeting_id?: string | null; title: string; start: string; end: string; kind: string };
export type DayPlan = { date: string; available_minutes: number; meeting_minutes: number; planned_minutes: number; required_minutes: number; overloaded: boolean; blocks: PlanBlock[]; unscheduled: Task[]; warnings: string[]; working_day: boolean };
export type Activity = { at: string; text: string; task_id?: string | null; actor: string };
export type Dashboard = {
  greeting: string; local_time: string; briefing: string[]; next_up?: Task | null; priorities: Task[]; due_today: Task[]; overdue: Task[];
  review: Task[]; review_count: number; blocked: Task[]; waiting: Task[]; meetings: Meeting[]; meetings_needing_prep: number;
  followups_count: number; recent: Activity[]; plan: DayPlan; unread_notifications: number; attention_accounts: string[]; monitoring_paused: boolean;
};
export type Followups = {
  awaiting_my_reply: Task[]; people_to_contact: Task[]; waiting_on_others: Commitment[]; waiting_tasks: Task[];
  awaiting_approval: Task[]; blocked_by_others: Task[]; promises_without_evidence: Task[];
};
export type Notification = {
  id: string; fingerprint: string; category: string; priority: number; title: string; body: string; entity_type?: string | null; entity_id?: string | null;
  group_key?: string | null; state: string; deliver_after: string; delivered_at?: string | null; delivery_error?: string | null; read_at?: string | null;
  snoozed_until?: string | null; expires_at?: string | null; created_at: string;
};
export type Rule = { category: string; enabled: boolean; desktop: boolean; lead_minutes: number; repeat_minutes: number; min_priority: number; bypass_quiet: boolean };
export type AiSettings = { provider: string; model: string; base_url: string; process_email: boolean; process_chat: boolean; process_calendar: boolean; redact_pii: boolean; max_chars_per_request: number; daily_request_limit: number; timeout_seconds: number };
export type Prefs = {
  onboarded: boolean; user_name: string; user_emails: string[]; user_aliases: string[]; timezone: string; working_days: number[];
  work_start: string; work_end: string; holidays: string[]; quiet_hours_enabled: boolean; quiet_start: string; quiet_end: string;
  planning_style: string; monitoring_paused: boolean; notifications_paused_until?: string | null; sync_interval_minutes: number;
  max_notifications_per_hour: number; digest_enabled: boolean; digest_time: string; eod_review_enabled: boolean; eod_review_time: string;
  meeting_prep_buffer_minutes: number; vip_emails: string[]; extraction_mode: string; auto_accept_threshold: number; review_everything: boolean;
  retention_body_days: number; retention_item_days: number; autostart: boolean; start_minimized: boolean; close_to_tray: boolean;
  initial_sync_days: number; google_client_id: string; microsoft_client_id: string; microsoft_tenant: string; slack_client_id: string; ai: AiSettings;
};
export type Provider = { id: string; name: string; category: string; support: string; streams: string[]; description: string; permissions: string[]; setup: string };
export type Account = {
  id: string; provider: string; category: string; display_name: string; email?: string | null; status: string; status_detail?: string | null;
  scopes?: string | null; enabled_streams: string[]; connected_at?: string | null; last_sync_at?: string | null; last_success_at?: string | null;
  next_sync_at?: string | null; backoff_until?: string | null; error_count: number; items_synced: number;
};
export type Integrations = { providers: Provider[]; accounts: Account[]; google_configured: boolean; google_secret_saved: boolean; microsoft_configured: boolean; slack_configured: boolean; keychain_ok: boolean };
export type Doc = { id: string; provider: string; name: string; mime_type?: string | null; url?: string | null; modified_at?: string | null; modified_by?: string | null };
export type SourceItem = {
  id: string; provider: string; kind: string; sender_name?: string | null; sender_email?: string | null; to: string[]; cc: string[];
  subject?: string | null; body_text?: string | null; snippet?: string | null; url?: string | null; sent_at: string; is_from_me: boolean;
};
export type TaskFilter = {
  statuses?: string[]; include_closed?: boolean; review_only?: boolean; search?: string; meeting_id?: string | null; parent_id?: string | null;
  top_level_only?: boolean; task_type?: string | null; sort?: string; limit?: number;
};
export type NewTask = Partial<Omit<Task, "people">> & { title: string; people?: Person[] };
export type TaskPatch = {
  title?: string; description?: string; task_type?: string; my_role?: string; owner_name?: string; priority?: number;
  due_at?: string | null; project?: string; expected_deliverable?: string; completion_criteria?: string; meeting_id?: string | null;
  effort_minutes?: number; recurrence?: string | null; counterpart_email?: string;
};

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return Promise.reject(new Error("Kept must be run as the desktop app."));
  return invoke<T>(cmd, args);
}

export const api = {
  dashboard: () => call<Dashboard>("get_dashboard"),
  dayPlan: () => call<DayPlan>("get_day_plan"),
  tasks: (filter: TaskFilter) => call<Task[]>("list_tasks", { filter }),
  task: (id: string) => call<TaskDetail>("get_task", { id }),
  createTask: (task: NewTask) => call<string>("create_task", { task }),
  updateTask: (id: string, patch: TaskPatch) => call<void>("update_task", { id, patch }),
  setStatus: (id: string, newStatus: string, reason?: string) => call<void>("set_task_status", { id, newStatus, reason }),
  review: (action: string, ids: string[], mergeInto?: string, reason?: string) =>
    call<number>("review_tasks", { req: { action, ids, merge_into: mergeInto, reason } }),
  undo: (auditId: string) => call<void>("undo_change", { auditId }),
  deleteTask: (id: string) => call<void>("delete_task", { id }),
  dependency: (id: string, dependsOn: string, remove = false) => call<void>("add_dependency", { id, dependsOn, remove }),
  source: (id: string) => call<SourceItem | null>("get_source", { id }),
  meetings: (days: number) => call<Meeting[]>("list_meetings", { days }),
  briefing: (id: string) => call<Briefing>("get_briefing", { id }),
  aiBriefing: (id: string) => call<string>("ai_briefing", { id }),
  followups: () => call<Followups>("list_followups"),
  setCommitment: (id: string, newStatus: string) => call<void>("set_commitment", { id, newStatus }),
  notifications: (scope: string) => call<Notification[]>("list_notifications", { scope }),
  notificationAction: (id: string | null, action: string, minutes?: number) => call<void>("notification_action", { id, action, minutes }),
  rules: () => call<Rule[]>("get_rules"),
  saveRule: (rule: Rule) => call<void>("save_rule", { rule }),
  testNotification: () => call<void>("test_notification"),
  prefs: () => call<Prefs>("get_prefs"),
  savePrefs: (prefs: Prefs) => call<Prefs>("save_prefs", { prefs }),
  integrations: () => call<Integrations>("get_integrations"),
  connect: (provider: string) => call<void>("connect_account", { provider }),
  syncNow: (accountId?: string) => call<string>("sync_now", { accountId }),
  disconnect: (id: string, deleteTasks: boolean) => call<void>("disconnect_account", { id, deleteTasks }),
  setStreams: (id: string, streams: string[]) => call<void>("set_account_streams", { id, streams }),
  setSecret: (kind: string, value: string) => call<void>("set_secret", { kind, value }),
  hasSecret: (kind: string) => call<boolean>("has_secret", { kind }),
  testAi: () => call<string>("test_ai"),
  importFiles: () => call<string | null>("import_files"),
  loadSample: () => call<string>("load_sample"),
  exportData: () => call<string | null>("export_data"),
  backupData: () => call<string | null>("backup_data"),
  deleteAll: (confirm: string) => call<void>("delete_all_data", { confirm }),
  storage: () => call<any>("storage_info"),
  auditLog: (limit: number) => call<any[]>("audit_log", { limit }),
  aiUsage: () => call<any[]>("ai_usage"),
  openExternal: (url: string) => call<void>("open_external", { url }),
  openFolder: (which: "data" | "logs") => call<void>("open_data_folder", { which }),
  appInfo: () => call<any>("app_info"),
  connectToken: (provider: string, token: string, email?: string, site?: string) => call<string>("connect_token", { provider, token, email, site }),
  relatedDocuments: (taskId: string) => call<Doc[]>("related_documents", { taskId }),
  pushToTodoist: (taskId: string) => call<string>("push_to_todoist", { taskId, confirmed: true }),
  reanalyse: (days: number) => call<string>("reanalyse", { days }),
  setPaused: (monitoring?: boolean, notificationsMinutes?: number) => call<void>("set_paused", { monitoring, notificationsMinutes }),
};

export function onChanged(cb: () => void): Promise<UnlistenFn> {
  if (!inTauri) return Promise.resolve(() => {});
  return listen("kept://changed", () => cb());
}
export function onConnect(cb: (p: { ok: boolean; provider: string; account?: string; error?: string }) => void): Promise<UnlistenFn> {
  if (!inTauri) return Promise.resolve(() => {});
  return listen<any>("kept://connect", (e) => cb(e.payload));
}
