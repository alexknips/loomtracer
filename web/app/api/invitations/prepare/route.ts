import { cookies } from "next/headers";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { z } from "zod";
export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const { invitation } = z.object({ invitation: z.string().regex(/^[0-9a-f-]{36}\.[0-9a-f]{64}$/i) }).parse(await request.json());
    (await cookies()).set("nf_invitation", invitation, { httpOnly: true, secure: process.env.NODE_ENV === "production", sameSite: "lax", maxAge: 1800, path: "/" });
    return privateJson({ ok: true });
  } catch (error) { return apiError(error); }
}
