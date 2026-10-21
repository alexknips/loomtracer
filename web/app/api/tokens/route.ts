import { privateJson } from "@/lib/server/http";
const retired = () => privateJson({ error: "Manual tokens are retired. Connect your agent through browser authorization." }, { status: 410 });
export const GET = retired;
export const POST = retired;
