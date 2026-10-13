/** Inspect only the disposable test user's admission; never print credentials. */
import {readFileSync,writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
const dir=new URL('../../.neurofilament/local-beta/',import.meta.url);
const {email}=JSON.parse(readFileSync(new URL('google-invitation.json',dir),'utf8'));
const literal=s=>"'"+String(s).replaceAll("'","''")+"'";
const result=spawnSync('/opt/homebrew/bin/docker',['--context','colima-neurofilament','exec','-i','supabase_db_neurofilament-beta-test','psql','-U','postgres','-d','postgres','-Atq','-v','ON_ERROR_STOP=1'],{
 encoding:'utf8',input:`select jsonb_build_object(
 'verifiedGoogle',exists(select 1 from auth.identities i where i.user_id=u.id and i.provider='google' and i.identity_data->>'email_verified'='true'),
 'admission',a.status,'administrator',a.is_admin,
 'workspaces',(select count(*) from public.workspaces where owner_user_id=u.id),
 'memberships',(select count(*) from public.workspace_members where user_id=u.id),
 'humanActors',(select count(*) from public.actors where kind='human' and external_ref=u.id::text),
 'projects',(select count(*) from public.projects p join public.workspaces w on w.id=p.workspace_id where w.owner_user_id=u.id),
 'nodes',(select count(*) from public.nodes n join public.workspaces w on w.id=n.workspace_id where w.owner_user_id=u.id),
 'activeCodexConnections',(select count(*) from public.agent_connections c where c.user_id=u.id and c.revoked_at is null and c.client_name='Codex'),
 'oauthSessions',(select count(*) from auth.sessions s where s.user_id=u.id and s.oauth_client_id is not null)
 ) from auth.users u left join public.account_admissions a on a.user_id=u.id where lower(u.email)=lower(${literal(email)});`});
if(result.status!==0)throw new Error('Local verification query failed.');
if(!result.stdout.trim())throw new Error('The real Google test user has not signed in yet.');
const report={checkedAt:new Date().toISOString(),target:'disposable local stack',...JSON.parse(result.stdout)};
writeFileSync(new URL('google-report.json',dir),JSON.stringify(report,null,2),{mode:0o600});
console.log(JSON.stringify(report,null,2));
