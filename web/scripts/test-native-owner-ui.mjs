/** Functional owner-invitation UI check. Disposable local synthetic fixtures only. */
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {createClient} from '@supabase/supabase-js';
import {createServerClient} from '@supabase/ssr';
import {chromium,expect} from '@playwright/test';

const dir=new URL('../../.neurofilament/local-beta/',import.meta.url);
const credentials=JSON.parse(readFileSync(new URL('stack.json',dir),'utf8'));
assert.equal(credentials.API_URL,'http://127.0.0.1:55421','Refusing a non-local target');
const fixtures=JSON.parse(readFileSync(new URL('fixtures.json',dir),'utf8'));
assert.equal(fixtures.length,2,'Run the native fixture suite first');
for(const fixture of fixtures)assert.match(fixture.email,/^native-[ab]-.*@example\.test$/,'Only synthetic native fixtures are allowed');
const app='http://127.0.0.1:3108',key=credentials.PUBLISHABLE_KEY??credentials.ANON_KEY;
const checked=({data,error})=>{if(error)throw new Error(error.message);return data;};
const report={startedAt:new Date().toISOString(),target:app,checks:[],screenshots:[]};
const check=(name,condition)=>{assert.ok(condition,name);report.checks.push(name);console.log('PASS '+name);};
let browser,adminDb,previousOwnerStatus;
const createdInvitations=[];
try {
 browser=await chromium.launch();
 async function sessionFor(fixture) {
  const db=createClient(credentials.API_URL,key,{auth:{persistSession:false,autoRefreshToken:false}});
  const {session}=checked(await db.auth.signInWithPassword({email:fixture.email,password:fixture.password}));
  const cookies=[];
  const ssr=createServerClient(credentials.API_URL,key,{cookies:{getAll:()=>[],setAll:values=>cookies.push(...values)}});
  checked(await ssr.auth.setSession(session));
  const context=await browser.newContext();
  await context.addCookies(cookies.map(c=>({name:c.name,value:c.value,url:app,httpOnly:false,sameSite:'Lax'})));
  return {db,context,page:await context.newPage()};
 }
 const admin=await sessionFor(fixtures[0]);adminDb=admin.db;
 check('Synthetic administrator can manage invitations',checked(await adminDb.rpc('is_beta_admin'))===true);
 for(const [name,width] of [['desktop',1360],['mobile',390]]) {
  await admin.page.setViewportSize({width,height:1000});
  await admin.page.goto(app+'/settings/invitations');
  const workspaceName='Native '+name+' studio',email=`native-ui-${randomUUID()}@example.test`;
  await admin.page.getByLabel('Workspace name',{exact:true}).fill(workspaceName);
  await admin.page.getByLabel('Google email',{exact:true}).fill(email);
  const responsePromise=admin.page.waitForResponse(response=>response.url()===app+'/api/invitations'&&response.request().method()==='POST');
  await admin.page.getByRole('button',{name:'Create invitation link',exact:true}).click();
  const response=await responsePromise;
  check(`${name}: invitation form submits named workspace`,response.status()===200&&response.request().postDataJSON().workspaceName===workspaceName);
  const invitation=await response.json();createdInvitations.push(invitation.id);
  const prepared=admin.page.getByRole('region',{name:'Prepared owner invitation'});
  await expect(prepared).toBeVisible();
  await expect(prepared.getByText(workspaceName,{exact:true})).toBeVisible();
  await expect(prepared.getByRole('heading',{name:'Invitation for '+email})).toBeVisible();
  await expect(prepared.getByLabel('Send this invitation link')).toHaveValue(invitation.link);
  check(`${name}: prepared invitation identifies intended workspace and recipient`,true);
  check(`${name}: invitation UI fits viewport`,await admin.page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth));
  await prepared.scrollIntoViewIfNeeded();
  const screenshot=fileURLToPath(new URL(`owner-invitation-${name}.png`,dir));
  await admin.page.screenshot({path:screenshot,mask:[prepared.getByLabel('Send this invitation link')]});
  report.screenshots.push(screenshot);
  const revoke=await admin.context.request.delete(app+'/api/invitations/'+invitation.id,{headers:{origin:app}});
  check(`${name}: synthetic invitation can be revoked`,revoke.status()===200);
 }
 // The native suite ends by testing suspension. Restore only its synthetic B
 // identity for visual inspection, then put the exact prior status back.
 const listing=checked(await adminDb.rpc('list_beta_invitations'));
 previousOwnerStatus=listing.accounts.find(account=>account.user_id===fixtures[1].id)?.status;
 assert.ok(previousOwnerStatus,'Synthetic workspace owner must exist');
 if(previousOwnerStatus!=='active')checked(await adminDb.rpc('set_account_status',{target_user_id:fixtures[1].id,new_status:'active'}));
 const owner=await sessionFor(fixtures[1]);
 const workspace=checked(await owner.db.rpc('get_workspace_context'))[0];
 for(const [name,width] of [['desktop',1360],['mobile',390]]) {
  await owner.page.setViewportSize({width,height:1000});await owner.page.goto(app+'/onboarding');
  await expect(owner.page.getByLabel('Current workspace: '+workspace.workspace_name,{exact:true})).toBeVisible();
  await expect(owner.page.getByRole('heading',{name:'Your workspace is ready.'})).toBeVisible();
  check(`${name}: actual owner onboarding shows its current workspace`,workspace.membership_role==='owner');
  check(`${name}: invited owner has no administrator navigation`,await owner.page.getByRole('link',{name:'Invitations',exact:true}).count()===0);
  check(`${name}: owner onboarding fits viewport`,await owner.page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth));
  const screenshot=fileURLToPath(new URL(`owner-onboarding-${name}.png`,dir));
  await owner.page.screenshot({path:screenshot});report.screenshots.push(screenshot);
 }
 report.result='passed';
} catch(error) {report.result='failed';report.error=error.message;console.error(error.message);process.exitCode=1;}
finally {
 if(adminDb) {
  for(const invitationId of createdInvitations)checked(await adminDb.rpc('revoke_beta_invitation',{invitation_id:invitationId}));
  if(previousOwnerStatus&&previousOwnerStatus!=='active')checked(await adminDb.rpc('set_account_status',{target_user_id:fixtures[1].id,new_status:previousOwnerStatus}));
 }
 await browser?.close();report.finishedAt=new Date().toISOString();
 writeFileSync(new URL('native-owner-ui-report.json',dir),JSON.stringify(report,null,2),{mode:0o600});
 console.log(JSON.stringify({result:report.result,checks:report.checks.length,report:fileURLToPath(new URL('native-owner-ui-report.json',dir))}));
}
