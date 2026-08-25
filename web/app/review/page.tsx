import { AppShell } from "@/components/app-shell";
import { ReviewQueue } from "@/components/review-queue";
import { requireOwner } from "@/lib/server/auth";
import { getReviewRecords } from "@/lib/server/data";

export const dynamic = "force-dynamic";

export default async function ReviewPage() {
  const owner = await requireOwner();
  const records = await getReviewRecords(owner.workspaceId);
  return <AppShell active="Review"><ReviewQueue initialRecords={records} /></AppShell>;
}
