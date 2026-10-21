/** HTTP intake through anonymous PostgREST plus native admin/member boundaries. Local only. */
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {randomUUID,randomBytes} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {createClient} from '@supabase/supabase-js';
const dir=new URL('../../.neurofilament/local-beta/',import.meta.url);
const credentials=JSON.parse(readFileSync(new URL('stack.json',dir),'utf8'));
assert.equal(credentials.API_URL,'http://127.0.0.1:55421','Refusing non-local database');
const owner=JSON.parse(readFileSync(new URL('fixtures.json',dir),'utf8'))[0];
assert.match(owner.email,/^native-.*@example\.test$/,'Only synthetic native fixtures are allowed');
const key=credentials.PUBLISHABLE_KEY??credentials.ANON_KEY,app='http://127.0.0.1:3108';
const client=()=>createClient(credentials.API_URL,key,{auth:{persistSession:false,autoRefreshToken:false}});
const checked=({data,error})=>{if(error)throw new Error(error.message);return data;};
const literal=s=>"'"+String(s).replaceAll("'","''")+"'";
function sql(statement){const result=spawnSync('/opt/homebrew/bin/docker',['--context','colima-neurofilament','exec','-i','supabase_db_neurofilament-beta-test','psql','-U','postgres','-d','postgres','-Atq','-v','ON_ERROR_STOP=1'],{input:statement,encoding:'utf8'});if(result.status!==0)throw new Error('Local fixture SQL failed');return result.stdout.trim();}
const report={result:'running',checks:0};const check=condition=>{assert.ok(condition);report.checks++;};
try {
 const admin=client();checked(await admin.auth.signInWithPassword({email:owner.email,password:owner.password}));
 check(checked(await admin.rpc('is_beta_admin'))===true);
 const email=`intake-${randomUUID()}@example.test`;
 const response=await fetch(app+'/api/access-requests',{method:'POST',headers:{origin:app,'content-type':'application/json'},body:JSON.stringify({email,use_case:'Native HTTP request to test shared decision memory.',website:''})});
 check(response.status===202);check((await response.json()).ok===true);
 const id=sql(`select id from private.access_requests where email=${literal(email)} and status='pending'`);check(Boolean(id));
 const anon=client();check(Boolean((await anon.rpc('list_access_requests')).error));
 check(Boolean((await anon.from('access_requests').select('*')).error));
 const requests=checked(await admin.rpc('list_access_requests'));check(requests.some(r=>r.id===id));
 const workspaceName='Native access request studio';
 const invitation=checked(await admin.rpc('approve_access_request',{request_id:id,invited_workspace_name:workspaceName}));
 check(invitation.kind==='owner'&&invitation.email===email&&invitation.workspace_name===workspaceName&&invitation.secret.length===64&&!invitation.secret_hash);
 check(sql(`select status from private.access_requests where id=${literal(id)}`)==='approved');
 // A separate native human joins the admin's team and must never gain queue access.
 const member={id:randomUUID(),email:`native-intake-member-${randomUUID()}@example.test`,password:randomBytes(32).toString('base64url')};
 const memberInvitation=checked(await admin.rpc('create_workspace_invitation',{invited_email:member.email}));
 sql(`insert into auth.users(id,instance_id,aud,role,email,encrypted_password,email_confirmed_at,created_at,updated_at,confirmation_token,recovery_token,email_change,email_change_token_new,raw_app_meta_data,raw_user_meta_data)
 values(${literal(member.id)},'00000000-0000-0000-0000-000000000000','authenticated','authenticated',${literal(member.email)},extensions.crypt(${literal(member.password)},extensions.gen_salt('bf')),now(),now(),now(),'','','','','{"provider":"google","providers":["google"]}','{}');
 insert into auth.identities(id,user_id,provider_id,provider,identity_data,created_at,updated_at)
 values(gen_random_uuid(),${literal(member.id)},${literal(member.id)},'google',jsonb_build_object('sub',${literal(member.id)},'email',${literal(member.email)},'email_verified',true),now(),now());`);
 const reviewer=client();checked(await reviewer.auth.signInWithPassword({email:member.email,password:member.password}));
 checked(await reviewer.rpc('redeem_beta_invitation',{invitation_id:memberInvitation.id,invitation_secret:memberInvitation.secret}));
 check(checked(await reviewer.rpc('get_workspace_context'))[0].membership_role==='reviewer');
 check(Boolean((await reviewer.rpc('list_access_requests')).error));
 check(Boolean((await reviewer.rpc('approve_access_request',{request_id:id})).error));
 checked(await admin.rpc('remove_workspace_member',{target_user_id:member.id}));
 checked(await admin.rpc('revoke_beta_invitation',{invitation_id:invitation.id}));
 report.result='passed';
} catch(error){report.result='failed';report.error=error.message;process.exitCode=1;}
writeFileSync(new URL('native-intake-report.json',dir),JSON.stringify(report,null,2),{mode:0o600});
console.log(JSON.stringify(report));
