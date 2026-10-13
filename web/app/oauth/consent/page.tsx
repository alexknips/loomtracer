import { redirect } from "next/navigation";
import { getOwnerContext } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { ConsentForm } from "@/components/consent-form";
import { OAUTH_AUTHORIZATION_ID } from "@/lib/server/auth-policy";
export const dynamic="force-dynamic";
export default async function ConsentPage({searchParams}:{searchParams:Promise<{authorization_id?:string}>}) {
  const {authorization_id:id}=await searchParams;
  if(!id||!OAUTH_AUTHORIZATION_ID.test(id)) return <main className="login-page"><p>This authorization request is unavailable. Restart the connection from your agent.</p></main>;
  const owner=await getOwnerContext();
  if(!owner) redirect(`/login?next=${encodeURIComponent(`/oauth/consent?authorization_id=${id}`)}`);
  const {data,error}=await (await createClient()).auth.oauth.getAuthorizationDetails(id);
  if(error||!data) return <main className="login-page"><p>This authorization request expired or is unavailable. Restart the connection from your agent.</p></main>;
  if("redirect_url" in data) redirect(data.redirect_url);
  if(data.user.id!==owner.user.id) return <main className="login-page"><p>This request belongs to another sign-in session.</p></main>;
  return <main className="login-page"><ConsentForm authorizationId={id} clientName={data.client.name} redirectUri={data.redirect_uri} email={owner.user.email??""} scope={data.scope} userId={owner.user.id} workspaceId={owner.workspaceId} workspaceName={owner.workspaceName} membershipRole={owner.membershipRole}/></main>;
}
