import { sessionTitle } from "@/lib/format";
import type { SessionItem } from "@/lib/types";

export type SessionActionTarget = {
  providerId: string;
  sessionId: string;
  title?: string | null;
  workspace?: string | null;
};

export function targetFromSession(session: SessionItem): SessionActionTarget {
  return {
    providerId: session.provider_id,
    sessionId: session.session_id,
    title: sessionTitle(session),
    workspace: session.project_dir,
  };
}
