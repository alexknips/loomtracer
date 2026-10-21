import { privateJson } from "@/lib/server/http";
export async function DELETE() { return privateJson({ error: "Manual tokens are retired." }, { status: 410 }); }
