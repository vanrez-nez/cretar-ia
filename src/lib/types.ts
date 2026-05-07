export type InteractionMode = "push_to_talk" | "toggle";

export type AppLanguage = "system" | "en" | "es";

export type PermissionState =
  | "granted"
  | "denied"
  | "not_determined"
  | "restricted"
  | "unsupported"
  | "unknown";

export interface PermissionsStatus {
  microphone: PermissionState;
  accessibility: PermissionState;
}
