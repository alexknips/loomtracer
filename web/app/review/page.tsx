import { redirect } from "next/navigation";

export const dynamic = "force-dynamic";

// Existing bookmarks open the new append-only review and commitment interface.
export default function ReviewPage() { redirect("/graph"); }
