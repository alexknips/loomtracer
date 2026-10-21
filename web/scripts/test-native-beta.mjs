/** Native GoTrue + PostgREST + browser OAuth. Only the disposable loopback stack. */
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,readdirSync} from 'node:fs';
import {randomUUID,randomBytes,createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {createClient} from '@supabase/supabase-js';
import {createServerClient} from '@supabase/ssr';
import {chromium,expect} from '@playwright/test';

// The isolated option has separate ports, container names and data. It never
// resets or reuses the older developer stack.
const isolated=process.env.NATIVE_REASONING_ISOLATED==='1';
const dir=new URL(isolated?'../../.neurofilament/reasoning-native/':'../../.neurofilament/local-beta/',import.meta.url);
const credentials=JSON.parse(readFileSync(new URL('stack.json',dir),'utf8'));
const base=credentials.API_URL, app=isolated?'http://127.0.0.1:3118':'http://127.0.0.1:3108', resource=app+'/api/mcp';
assert.equal(base,isolated?'http://127.0.0.1:58421':'http://127.0.0.1:55421','Refusing a non-local target');
const key=credentials.PUBLISHABLE_KEY??credentials.ANON_KEY;
const docker=process.env.DOCKER_BIN??'/opt/homebrew/bin/docker';
const context='colima-neurofilament',container=isolated?'supabase_db_loomtracer-reasoning-native':'supabase_db_neurofilament-beta-test';
const literal=s=>"'"+String(s).replaceAll("'","''")+"'";
function sql(statement) {
 const result=spawnSync(docker,['--context',context,'exec','-i',container,'psql','-U','postgres','-d','postgres','-Atq','-v','ON_ERROR_STOP=1'],{input:statement,encoding:'utf8'});
 if(result.status!==0)throw new Error('Local fixture SQL failed: '+result.stderr.split('\n').filter(l=>!l.startsWith('LINE ')).join('\n'));
 return result.stdout.trim();
}
const report={startedAt:new Date().toISOString(),target:base,checks:[],googleProvider:'Not exercised by synthetic human fixtures'};
const check=(name,condition)=>{assert.ok(condition,name);report.checks.push(name);console.log('PASS '+name);};
async function json(url,options={}) {
 const response=await fetch(url,options);const text=await response.text();
 let body;try{body=JSON.parse(text);}catch{body={message:text.slice(0,150)};}
 return {response,body};
}
const client=token=>createClient(base,key,{accessToken:async()=>token,auth:{persistSession:false,autoRefreshToken:false}});
const checked=({data,error})=>{if(error)throw new Error(error.message);return data;};
const nativeTools=['list_projects','get_reasoning_brief','list_reasoning_datasets','search_reasoning','expand_reasoning','get_reconstruction_packet','capture_reasoning','revise_extraction','change_commitment'].sort();
async function mcpCall(agent,name,args) {
 const result=await json(resource,{method:'POST',headers:{Authorization:`Bearer ${agent.tokens.access_token}`,Accept:'application/json, text/event-stream','Content-Type':'application/json','MCP-Protocol-Version':'2025-06-18'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/call',params:{name,arguments:args}})});
 return {...result,call:result.body.result,value:result.body.result?.structuredContent?.result};
}
async function nativeDiscovery(u) {
 const result=await json(resource,{method:'POST',headers:{Authorization:`Bearer ${u.agent.tokens.access_token}`,Accept:'application/json, text/event-stream','Content-Type':'application/json','MCP-Protocol-Version':'2025-06-18'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/list',params:{}})});
 assert.equal(result.response.status,200);assert.deepEqual(result.body.result.tools.map(tool=>tool.name).sort(),nativeTools);
 check(`User ${u.label}: fresh discovery exposes only native reasoning tools`,true);
}
function nativeDelta(stored,actor,content) {
 const key=`native:${randomUUID()}`,now=new Date().toISOString(),sourceId=`${key}:source`,contextId=`${key}:context`,entryId=`${key}:entry`;
 return {actors:[stored.bundle.actors.find(item=>item.id===actor.id)??actor],contexts:[{id:contextId,title:'Disposable native authorization test',kind:'discussion'}],
  sources:[{id:sourceId,originalId:sourceId,actorId:actor.id,role:actor.kind==='human'?'user':'assistant',content,occurredAt:now,loggedAt:now,order:Math.max(0,...stored.bundle.sources.map(source=>source.order))+1,sourceRef:key,attachmentIds:[],active:true,provenance:{kind:'authored'}}],
  entries:[{id:entryId,logicalId:entryId,version:1,contextId,labels:['observation'],provenance:{expressedBy:actor.id,recordedBy:actor.id,occurredAt:now,recordedAt:now,basis:'direct',sourceSpans:[{messageId:sourceId,start:0,end:content.length}]}}],
  run:{id:`${key}:run`,createdAt:now,recordedBy:actor.id,notes:'Disposable native capture; no commitment authorized.'}};
}
function datasetArgs(u,project=u.project) {return {target_workspace_id:u.workspace,target_project_id:project.id,target_dataset_id:`legacy:${u.workspace}:${project.id}`};}
async function readNative(u,project=u.project) {return checked(await u.db.rpc('reasoning_read_bundle',datasetArgs(u,project)));}
function importLocalLegacy(users) {
 const workspaceFilter=users.map(u=>literal(u.workspace)).join(',');
 sql("select private.reasoning_set_mode('read_only')");sql("select private.reasoning_set_mode('frozen')");
 const tables=['nodes','edges','actors','projects','audit_events'];
 const contents=JSON.parse(sql(`select jsonb_build_object(${tables.map(table=>`${literal(table)},(select coalesce(jsonb_agg(to_jsonb(t) order by t.id),'[]') from public.${table} t where workspace_id in (${workspaceFilter}))`).join(',')})`));
 const snapshot={snapshotId:`native-auth:${randomUUID()}`,createdAt:new Date().toISOString(),tables:contents};
 const snapshotPath=fileURLToPath(new URL(`reasoning-${randomUUID()}.json`,dir)),convertedPath=fileURLToPath(new URL(`reasoning-${randomUUID()}/`,dir));
 writeFileSync(snapshotPath,JSON.stringify(snapshot),{mode:0o600});
 const converted=spawnSync(process.execPath,['--experimental-strip-types',fileURLToPath(new URL('../../scripts/reasoning-convert.mjs',import.meta.url)),snapshotPath,convertedPath],{encoding:'utf8'});
 assert.equal(converted.status,0,'Local legacy converter failed: '+converted.stderr);
 for(const name of readdirSync(convertedPath).filter(name=>name.endsWith('.bundle.json'))) {
  const bundle=JSON.parse(readFileSync(`${convertedPath}/${name}`,'utf8')),wid=literal(bundle.namespace.workspaceId),pid=literal(bundle.namespace.projectId);
  sql(`select private.reasoning_import_legacy(${wid},${pid},${literal(JSON.stringify(bundle))}::jsonb,private.reasoning_legacy_fingerprint(${wid},${pid}),${literal(snapshot.snapshotId+':'+bundle.id)})`);
 }
 const preserved=JSON.parse(sql(`select coalesce(jsonb_agg(to_jsonb(t) order by t.id),'[]') from public.nodes t where workspace_id in (${workspaceFilter})`));
 assert.deepEqual(preserved,contents.nodes,'Local migration changed an original human or agent decision');
 sql("select private.reasoning_set_mode('reasoning_open')");
 check('Local-only frozen conversion preserves legacy decisions and opens native reasoning',true);
}
const fixtures=['A','B'].map(label=>({id:randomUUID(),email:`native-${label.toLowerCase()}-${randomUUID()}@example.test`,password:randomBytes(32).toString('base64url'),label}));
let browser;
try {
 // Preserve the legacy authorization regression before this test's explicit
 // cutover. Never reset a stack that already contains new reasoning history.
 assert.equal(sql('select count(*) from public.reasoning_datasets'),'0','Use a fresh disposable local stack; existing reasoning history must not be overwritten.');
 assert.equal(sql('select mode from private.reasoning_control where singleton'),'legacy_open','Native test must start before local reasoning cutover.');
 sql(`update private.beta_config set agents_enabled=true,invitations_enabled=true,mcp_resource=${literal(resource)};
 ${fixtures.map(u=>`insert into auth.users(id,instance_id,aud,role,email,encrypted_password,email_confirmed_at,created_at,updated_at,confirmation_token,recovery_token,email_change,email_change_token_new,raw_app_meta_data,raw_user_meta_data)
 values(${literal(u.id)},'00000000-0000-0000-0000-000000000000','authenticated','authenticated',${literal(u.email)},extensions.crypt(${literal(u.password)},extensions.gen_salt('bf')),now(),now(),now(),'','','','','{"provider":"google","providers":["google"]}','{}');
 insert into auth.identities(id,user_id,provider_id,provider,identity_data,created_at,updated_at)
 values(gen_random_uuid(),${literal(u.id)},${literal(u.id)},'google',jsonb_build_object('sub',${literal(u.id)},'email',${literal(u.email)},'email_verified',true),now(),now());`).join('\n')}
 insert into public.account_admissions(user_id,is_admin) values(${literal(fixtures[0].id)},true);
 -- Operator-only synthetic fixture; ordinary admission is invitation redemption.
 insert into public.workspaces(name,slug,owner_user_id) values('Native fixture team',${literal('native-'+fixtures[0].id)},${literal(fixtures[0].id)});
 insert into public.workspace_members(workspace_id,user_id,role) select id,owner_user_id,'owner' from public.workspaces where owner_user_id=${literal(fixtures[0].id)};
 insert into public.actors(workspace_id,kind,display_name,external_ref) select id,'human',${literal(fixtures[0].email)},owner_user_id::text from public.workspaces where owner_user_id=${literal(fixtures[0].id)};`);
 for(const u of fixtures) {
  const auth=createClient(base,key,{auth:{persistSession:false,autoRefreshToken:false}});
  u.session=checked(await auth.auth.signInWithPassword({email:u.email,password:u.password})).session;
  u.db=client(u.session.access_token);
 }
 const [a,b]=fixtures;
 check('Native human sessions use asymmetric signatures',fixtures.every(u=>JSON.parse(Buffer.from(u.session.access_token.split('.')[0],'base64url')).alg==='ES256'));
 const directSignup=createClient(base,key,{auth:{persistSession:false,autoRefreshToken:false}});
 const forbiddenEmail=`uninvited-${randomUUID()}@example.test`;
 const forbiddenSignup=await directSignup.auth.signUp({email:forbiddenEmail,password:randomBytes(32).toString('base64url')});
 check('Native signup hook blocks direct email signup',Boolean(forbiddenSignup.error)&&sql(`select count(*) from auth.users where email=${literal(forbiddenEmail)}`)==='0');
 check('Unadmitted native session cannot bootstrap',(await b.db.rpc('bootstrap_personal_workspace')).error!==null);
 a.workspace=checked(await a.db.rpc('bootstrap_personal_workspace'))[0].workspace_id;
 browser=await chromium.launch();
 const adminCookies=[];
 const adminSsr=createServerClient(base,key,{cookies:{getAll:()=>[],setAll:values=>adminCookies.push(...values)}});
 checked(await adminSsr.auth.setSession(a.session));
 a.context=await browser.newContext();
 a.cookies=adminCookies.map(c=>({name:c.name,value:c.value,url:app,httpOnly:false,sameSite:'Lax'}));
 await a.context.addCookies(a.cookies);a.page=await a.context.newPage();
 const workspaceName='Native independent studio';
 await a.page.goto(app+'/settings/invitations');
 await a.page.getByLabel('Workspace name',{exact:true}).fill(workspaceName);
 await a.page.getByLabel('Google email',{exact:true}).fill(b.email);
 const createdResponse=a.page.waitForResponse(response=>response.url()===app+'/api/invitations'&&response.request().method()==='POST');
 await a.page.getByRole('button',{name:'Create invitation link',exact:true}).click();
 const invitationResponse=await createdResponse;
 check('Administrator creates named owner invitation through browser form',invitationResponse.status()===200&&invitationResponse.request().postDataJSON().workspaceName===workspaceName);
 const invitationBody=await invitationResponse.json();
 const prepared=a.page.getByRole('region',{name:'Prepared owner invitation'});
 await expect(prepared).toBeVisible();
 await expect(prepared.getByText(workspaceName,{exact:true})).toBeVisible();
 const invitationToken=new URL(invitationBody.link).hash.slice(1);
 const [invitationId,invitationSecret]=invitationToken.split('.');
 check('Owner invitation link contains a one-time secret',/^[0-9a-f-]{36}\.[0-9a-f]{64}$/i.test(invitationToken));
 const provisioningState=()=>sql(`select jsonb_build_object(
  'admissions',(select count(*) from public.account_admissions where user_id=${literal(b.id)}),
  'workspaces',(select count(*) from public.workspaces where owner_user_id=${literal(b.id)}),
  'memberships',(select count(*) from public.workspace_members where user_id=${literal(b.id)}),
  'actors',(select count(*) from public.actors where kind='human' and external_ref=${literal(b.id)}),
  'invitation',(select jsonb_build_object('workspace',workspace_id,'redeemed_at',redeemed_at,'redeemed_by',redeemed_by) from private.beta_invitations where id=${literal(invitationId)}))`);
 const unredeemedState=provisioningState();
 check('Creating owner invitation does not provision its workspace',JSON.parse(unredeemedState).workspaces===0&&JSON.parse(unredeemedState).admissions===0);
 async function prepareInvitation(context,token) {
  const response=await context.request.post(app+'/api/invitations/prepare',{headers:{origin:app},data:{invitation:token}});
  assert.equal(response.status(),200,'Invitation prepare failed');
  const cookie=(await context.cookies()).find(c=>c.name==='nf_invitation');
  assert.ok(cookie?.httpOnly&&cookie.value===token,'Invitation must be held in an HttpOnly cookie');
  await context.addCookies([{name:'neurofilament-project',value:'stale-project',url:app}]);
 }
 const hasInvitationState=async context=>(await context.cookies()).some(c=>['nf_invitation','neurofilament-project'].includes(c.name));
 // Seed only the completed external-provider handoff. Native GoTrue still
 // verifies PKCE, issues the human session, and the real Next callback redeems
 // the prepared invitation and sets cookies. External Google remains a manual gate.
 async function nativeCallback(user,token) {
  const handoffCookies=[];
  const handoff=createServerClient(base,key,{cookies:{getAll:()=>[],setAll:values=>handoffCookies.push(...values)}});
  const handoffUrl=new URL(checked(await handoff.auth.signInWithOAuth({provider:'google',options:{skipBrowserRedirect:true}})).url);
  const handoffCode=randomUUID();
  sql(`insert into auth.flow_state(id,user_id,auth_code,code_challenge,code_challenge_method,provider_type,provider_access_token,provider_refresh_token,authentication_method,created_at,updated_at)
   values(gen_random_uuid(),${literal(user.id)},${literal(handoffCode)},${literal(handoffUrl.searchParams.get('code_challenge'))},'s256','google','','','oauth',now(),now());`);
  const context=await browser.newContext();
  await context.addCookies(handoffCookies.map(c=>({name:c.name,value:c.value,url:app,httpOnly:false,sameSite:'Lax'})));
  if(token)await prepareInvitation(context,token);
  const page=await context.newPage(),trace=[];
  page.on('response',response=>{if(response.request().isNavigationRequest())trace.push({origin:new URL(response.url()).origin,path:new URL(response.url()).pathname,error:new URL(response.url()).searchParams.get('error'),status:response.status()});});
  await page.goto(app+'/auth/callback?next=/onboarding&code='+handoffCode);
  return {context,page,trace};
 }
 for(const [name,path] of [['missing code','/auth/callback'],['invalid exchange','/auth/callback?code='+randomUUID()]]) {
  const context=await browser.newContext();await prepareInvitation(context,invitationToken);
  const response=await context.request.get(app+path,{maxRedirects:0});
  check(`Callback ${name} clears invitation and project cookies`,response.status()===307&&response.headers().location===app+'/login?error=callback'&&!await hasInvitationState(context));
  check(`Callback ${name} does not provision invitation`,provisioningState()===unredeemedState);
  await context.close();
 }
 const wrongSecret=invitationSecret.slice(0,-1)+(invitationSecret.endsWith('0')?'1':'0');
 for(const [name,user,token,closedGate] of [
  ['wrong Google account',a,invitationToken,false],
  ['incorrect invitation secret',b,invitationId+'.'+wrongSecret,false],
  ['closed invitation gate',b,invitationToken,true],
 ]) {
  if(closedGate)sql('update private.beta_config set invitations_enabled=false');
  let denied;
  try {denied=await nativeCallback(user,token);}
  finally {if(closedGate)sql('update private.beta_config set invitations_enabled=true');}
  check(`Owner callback rejects ${name}`,denied.page.url()===app+'/login?error=unauthorized');
  check(`Rejected ${name} creates no admission or workspace data`,provisioningState()===unredeemedState);
  check(`Rejected ${name} clears invitation and session`,!await hasInvitationState(denied.context)&&(await denied.context.request.get(app+'/api/session')).status()===401);
  if(user===a)check('Rejected wrong-account callback preserves existing signed-in session',(await a.context.request.get(app+'/api/session')).status()===200);
  await denied.context.close();
 }
 const ownerCallback=await nativeCallback(b,invitationToken);
 b.context=ownerCallback.context;b.page=ownerCallback.page;b.cookies=await b.context.cookies();
 const callbackSsr=createServerClient(base,key,{cookies:{getAll:()=>b.cookies.map(({name,value})=>({name,value})),setAll:()=>{}}});
 b.session=checked(await callbackSsr.auth.getSession()).session;
 assert.ok(b.session,'Successful callback must retain its native Auth session');
 b.db=client(b.session.access_token);
 writeFileSync(new URL('callback-trace.json',dir),JSON.stringify(ownerCallback.trace),{mode:0o600});
 const ownerContext=checked(await b.db.rpc('get_workspace_context'))[0];
 b.workspace=ownerContext.workspace_id;
 check('Native invitation PKCE callback reaches named owner onboarding',b.page.url()===app+'/onboarding'&&ownerContext.workspace_name===workspaceName&&ownerContext.membership_role==='owner');
 await expect(b.page.getByLabel('Current workspace: '+workspaceName,{exact:true})).toBeVisible();
 check('Successful callback clears invitation and project cookies',!await hasInvitationState(b.context));
 const provisionedState=provisioningState(),provisioned=JSON.parse(provisionedState);
 check('Owner callback provisions exactly one separate workspace and human actor',a.workspace!==b.workspace&&provisioned.admissions===1&&provisioned.workspaces===1&&provisioned.memberships===1&&provisioned.actors===1&&provisioned.invitation.workspace===b.workspace);
 check('Invited workspace owner is not a beta administrator',checked(await b.db.rpc('is_beta_admin'))===false);
 for(const table of ['projects','nodes','edges'])check(`Fresh owner workspace has no ${table}`,checked(await b.db.from(table).select('*')).length===0);
 check('Owner has no inherited agent grants',checked(await b.db.from('agent_connections').select('*')).length===0);
 const redemption=await Promise.all([b.db.rpc('redeem_beta_invitation',{invitation_id:invitationId,invitation_secret:invitationSecret}),b.db.rpc('redeem_beta_invitation',{invitation_id:invitationId,invitation_secret:invitationSecret})]);
 check('Concurrent owner redemption replay preserves one named workspace',redemption.every(r=>checked(r)[0].workspace_id===b.workspace&&checked(r)[0].workspace_name===workspaceName)&&provisioningState()===provisionedState);
 check('Owner cannot issue beta owner invitations',(await b.context.request.post(app+'/api/invitations',{headers:{origin:app},data:{email:`denied-${randomUUID()}@example.test`,workspaceName:'Forbidden'}})).status()===403);
 for(const u of fixtures) {
  u.project=checked(await u.db.rpc('save_project',{target_workspace_id:u.workspace,target_project_id:randomUUID(),project_ref:'project:same',project_name:'Same private project',project_description:'',parent_project_id:null}));
 }
 check('Identical project names are private',fixtures.every(u=>u.project.workspace_id===u.workspace));
 // Local Supabase exposes OIDC discovery but currently lacks the hosted
 // gateway's RFC 8414 rewrite. MCP clients must support this OIDC fallback.
 let discovery=await json(base+'/.well-known/oauth-authorization-server/auth/v1');
 if(discovery.response.status===404)discovery=await json(base+'/auth/v1/.well-known/openid-configuration');
 assert.equal(discovery.response.status,200);
 const metadata=discovery.body;
 assert.equal(metadata.issuer,base+'/auth/v1');
 check('Native discovery advertises PKCE and registration',Boolean(metadata.registration_endpoint)&&metadata.code_challenge_methods_supported.includes('S256'));
 const challenge=await json(resource,{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'});
 check('MCP advertises protected-resource discovery',challenge.response.status===401&&challenge.response.headers.get('www-authenticate').includes('/.well-known/oauth-protected-resource/api/mcp'));
 const protectedResource=await json(app+'/.well-known/oauth-protected-resource/api/mcp');
 check('Protected resource binds the canonical endpoint and issuer',protectedResource.body.resource===resource&&protectedResource.body.authorization_servers[0]===metadata.issuer);
 const returningCallback=await nativeCallback(a);
 check('Returning native PKCE callback preserves session through onboarding redirect',returningCallback.page.url()===app+'/onboarding');
 await returningCallback.context.close();
 const callback=app+'/native-test-callback';
 async function authorize(u,allowWrites=true,existingClient=null) {
  let registration=existingClient;
  if(!registration) {
   const result=await json(metadata.registration_endpoint,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({client_name:`Native test ${u.label}`,redirect_uris:[callback],grant_types:['authorization_code','refresh_token'],response_types:['code'],token_endpoint_auth_method:'none'})});
   assert.equal(result.response.status,201,JSON.stringify(result.body));registration=result.body;
  }
  const verifier=randomBytes(48).toString('base64url'), state=randomUUID();
  const url=new URL(metadata.authorization_endpoint);
  url.search=new URLSearchParams({client_id:registration.client_id,redirect_uri:callback,response_type:'code',scope:'openid',state,resource,code_challenge:createHash('sha256').update(verifier).digest('base64url'),code_challenge_method:'S256'}).toString();
  let returned;
  await u.page.route(callback+'**',route=>{returned=new URL(route.request().url());return route.fulfill({contentType:'text/html',body:'Authorization callback received.'});});
  await u.page.goto(url.href);
  try {await u.page.getByRole('button',{name:'Authorize agent'}).waitFor({timeout:20000});}
  catch(error) {
   await u.page.screenshot({path:fileURLToPath(new URL('consent-failure.png',dir))});
   writeFileSync(new URL('consent-failure.txt',dir),await u.page.locator('body').innerText(),{mode:0o600});
   throw error;
  }
  await expect(u.page.getByRole('checkbox',{name:'Allow reasoning capture'})).toBeVisible();
  await expect(u.page.getByText('This connection alone does not authorize commitments.',{exact:false})).toContainText('A workspace owner can separately grant this named agent');
  const authorizationId=new URL(u.page.url()).searchParams.get('authorization_id');
  const beforeGrant=checked(await u.db.from('agent_connections').select('id,granted_at,revoked_at').eq('client_id',registration.client_id));
  const wrongWorkspace=await u.context.request.post(app+'/api/oauth/consent',{headers:{origin:app},data:{authorizationId,approve:true,allowWrites,expectedUserId:u.id,expectedWorkspaceId:randomUUID()}});
  check(`User ${u.label}: stale workspace consent rejected`,wrongWorkspace.status()===403);
  const wrongIdentity=await u.context.request.post(app+'/api/oauth/consent',{headers:{origin:app},data:{authorizationId,approve:true,allowWrites,expectedUserId:randomUUID(),expectedWorkspaceId:u.workspace}});
  check(`User ${u.label}: stale identity consent rejected`,wrongIdentity.status()===403);
  check(`User ${u.label}: stale consent creates no grant`,JSON.stringify(checked(await u.db.from('agent_connections').select('id,granted_at,revoked_at').eq('client_id',registration.client_id)))===JSON.stringify(beforeGrant));
  if(!allowWrites)await u.page.getByRole('checkbox').uncheck();
  const approval=u.page.waitForResponse('**/api/oauth/consent');
  await u.page.getByRole('button',{name:'Authorize agent'}).click();
  const approved=await approval;
  assert.equal(approved.status(),200,approved.status()===200?undefined:await approved.text());
  check(`User ${u.label}: displayed workspace accompanies consent`,approved.request().postDataJSON().expectedWorkspaceId===u.workspace);
  check(`User ${u.label}: displayed identity accompanies consent`,approved.request().postDataJSON().expectedUserId===u.id);
  await u.page.waitForURL(callback+'**');assert.equal(returned.searchParams.get('state'),state);
  const tokens=await json(metadata.token_endpoint,{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'authorization_code',client_id:registration.client_id,code:returned.searchParams.get('code'),redirect_uri:callback,code_verifier:verifier,resource})});
  assert.equal(tokens.response.status,200,JSON.stringify(tokens.body));
  return {registration,tokens:tokens.body,claims:JSON.parse(Buffer.from(tokens.body.access_token.split('.')[1],'base64url'))};
 }
 for(const u of fixtures) {
  u.agent=await authorize(u);
  check(`User ${u.label}: browser consent and native code exchange`,u.agent.claims.aud===resource&&u.agent.claims.client_id===u.agent.registration.client_id&&Boolean(u.agent.claims.nf_connection_id));
  const agent=client(u.agent.tokens.access_token);
  const node=checked(await agent.rpc('capture_oauth_node',{target_node_id:'D-same',target_type:'Decision',target_status:'candidate',target_title:`PRIVATE-${u.label}-SENTINEL`,target_summary:'',target_body:'',target_scope_type:null,target_scope_refs:[],target_confidence:null,target_reversibility:null,target_consequence:null,target_reconsider_conditions:['Changed'],target_metadata:{},target_project_id:u.project.id}));
  check(`User ${u.label}: attributed proposal through RLS`,node.node.workspace_id===u.workspace);
  check(`User ${u.label}: agent cannot ratify`,Boolean((await agent.rpc('ratify_node',{target_workspace_id:u.workspace,target_node_id:'D-same',supplied_reconsider_conditions:['Changed']})).error));
 }
 for(const u of fixtures) {
  const foreign=fixtures.find(other=>other!==u), agent=client(u.agent.tokens.access_token);
  const projectArgs={target_workspace_id:u.workspace,target_project_id:randomUUID(),project_ref:'project:forbidden',project_name:'Forbidden',project_description:'',parent_project_id:null};
  const organization={target_workspace_id:u.workspace,target_node_id:'D-same',owning_project_id:u.project.id,inheritance:'local',expected_updated_at:new Date().toISOString()};
  for(const [name,args] of [
   ['bootstrap_personal_workspace',{}],['save_project',projectArgs],['organize_node',organization],
   ['reject_node',{target_workspace_id:u.workspace,target_node_id:'D-same',rejection_reason:'Denied'}],
   ['create_beta_invitation',{invited_email:'forbidden@example.test'}],['list_beta_invitations',{}],
   ['approve_agent_connection',{oauth_client_id:randomUUID(),display_name:'Forbidden',allow_writes:true}],
   ['disconnect_agent',{connection_id:u.agent.claims.nf_connection_id}],
   ['set_account_status',{target_user_id:foreign.id,new_status:'suspended'}],
   ['beta_access_token_hook',{event:{}}],['before_beta_user_created',{event:{}}],
  ])check(`User ${u.label}: direct agent ${name} denied`,Boolean((await agent.rpc(name,args)).error));
  for(const [label,db] of [['human',u.db],['agent',agent]]) {
   for(const table of ['workspaces','projects','nodes','edges','actors','audit_events','workspace_members','agent_connections']) {
    const rows=checked(await db.from(table).select('*'));
    check(`User ${u.label}: ${label} unfiltered ${table}`,rows.every(row=>(table==='workspaces'?row.id:row.workspace_id)===u.workspace));
   }
   const foreignRows=await db.from('projects').select('*').eq('id',foreign.project.id);
   const missingRows=await db.from('projects').select('*').eq('id',randomUUID());
   check(`User ${u.label}: ${label} guessed project indistinguishable`,JSON.stringify(foreignRows.data)===JSON.stringify(missingRows.data)&&foreignRows.status===missingRows.status);
   check(`User ${u.label}: ${label} pagination remains private`,checked(await db.from('nodes').select('*').range(1,2)).length===0);
   const subgraph=checked(await db.rpc('get_graph_subgraph',{target_workspace_id:foreign.workspace,root_node_id:'D-same'}));
   check(`User ${u.label}: ${label} foreign subgraph empty`,subgraph.nodes.length===0&&subgraph.edges.length===0);
  }
  check(`User ${u.label}: supplied foreign workspace denied`,Boolean((await u.db.rpc('save_project',{...projectArgs,target_workspace_id:foreign.workspace})).error));
  check(`User ${u.label}: cross-workspace parent denied`,Boolean((await u.db.rpc('save_project',{...projectArgs,parent_project_id:foreign.project.id})).error));
  const ownNode=checked(await u.db.from('nodes').select('*').eq('id','D-same').single());
  check(`User ${u.label}: cross-workspace inheritance denied`,Boolean((await u.db.rpc('organize_node',{...organization,owning_project_id:foreign.project.id,expected_updated_at:ownNode.updated_at})).error));
  check(`User ${u.label}: direct cross-workspace edge denied`,Boolean((await u.db.from('edges').insert({workspace_id:foreign.workspace,id:'E-denied',source_id:'D-same',edge_type:'informs',target_id:'D-same'})).error));
  const params={...projectArgs,target_project_id:randomUUID(),project_ref:'project:child',project_name:'Child',parent_project_id:u.project.id};
  u.child=checked(await u.db.rpc('save_project',params));
  checked(await u.db.rpc('ratify_node',{target_workspace_id:u.workspace,target_node_id:'D-same',supplied_reconsider_conditions:['Changed'],expected_updated_at:ownNode.updated_at}));
  const ratified=checked(await u.db.from('nodes').select('*').eq('id','D-same').single());
  checked(await u.db.rpc('organize_node',{...organization,inheritance:'descendants',expected_updated_at:ratified.updated_at}));
 }
 for(const u of fixtures) {
  for(const actor of [u.db,client(u.agent.tokens.access_token)]) {
   const nodes=checked(await actor.from('nodes').select('*'));
   check(`User ${u.label}: unfiltered REST isolation (${report.checks.length})`,nodes.length===1&&nodes[0].title===`PRIVATE-${u.label}-SENTINEL`);
  }
  const response=await u.context.request.get(app+'/api/export?format=jsonl');
  const text=await response.text();
  check(`User ${u.label}: website export isolation`,response.status()===200&&text.includes(`PRIVATE-${u.label}-SENTINEL`)&&!text.includes(`PRIVATE-${u.label==='A'?'B':'A'}-SENTINEL`));
  await nativeDiscovery(u);
  const brief=await mcpCall(u.agent,'get_reasoning_brief',{task:'Inherited private guidance before local migration',project_ref:'project:child'});
  check(`User ${u.label}: pre-cutover native brief cannot report complete empty guidance`,brief.response.status===200&&!brief.call.isError&&brief.value.complete===false&&brief.value.coverage_complete===false&&brief.value.warnings.some(warning=>warning.includes('legacy_open')));
  for(const path of ['/api/graph','/api/search?q=PRIVATE','/api/export?format=markdown']) {
   const response=await u.context.request.get(app+path+(path.includes('?')?'&':'?')+'workspace_id='+fixtures.find(other=>other!==u).workspace);
   const text=await response.text();
   check(`User ${u.label}: website ${path} private and uncached`,response.status()===200&&!text.includes(`PRIVATE-${u.label==='A'?'B':'A'}-SENTINEL`)&&response.headers()['cache-control'].includes('private, no-store'));
  }
  const forgedParts=u.agent.tokens.access_token.split('.');
  forgedParts[1]=Buffer.from(JSON.stringify({...u.agent.claims,sub:fixtures.find(other=>other!==u).id})).toString('base64url');
  const forged=await json(resource,{method:'POST',headers:{Authorization:`Bearer ${forgedParts.join('.')}`,'Content-Type':'application/json'},body:'{}'});
  check(`User ${u.label}: forged foreign-identity token rejected`,forged.response.status===401);
 }
 async function verifyAccountSwitch() {
 const switchContext=await browser.newContext();
 await switchContext.addCookies(a.cookies);
 const switchPage=await switchContext.newPage();
 await switchPage.goto(app+'/graph?project=project:same');
 await expect(switchPage.getByText('PRIVATE-A-SENTINEL',{exact:true}).first()).toBeVisible();
 await switchPage.goto(app+'/projects');
 const switchAuth=createClient(base,key,{auth:{persistSession:false,autoRefreshToken:false}});
 const switchSession=checked(await switchAuth.auth.signInWithPassword({email:b.email,password:b.password})).session;
 const switchCookies=[];
 const switchSsr=createServerClient(base,key,{cookies:{getAll:()=>[],setAll:values=>switchCookies.push(...values)}});
 checked(await switchSsr.auth.setSession(switchSession));
 await switchContext.clearCookies();await switchContext.addCookies(switchCookies.map(c=>({name:c.name,value:c.value,url:app,httpOnly:false,sameSite:'Lax'})));
 await switchPage.goBack();
 await expect(switchPage.getByRole('combobox',{name:'Reasoning project',exact:true})).toBeVisible();
 assert.equal((await (await switchContext.request.get(app+'/api/session')).json()).userId,b.id);
 await expect(switchPage.getByText('PRIVATE-A-SENTINEL',{exact:true}).first()).not.toBeVisible();
 // Account changes clear the selected project; deliberately select the new
 // account's canonical reference before expecting its private history.
 await switchPage.goto(app+'/graph?project=project:same');
 try {await expect(switchPage.getByText('PRIVATE-B-SENTINEL',{exact:true}).first()).toBeVisible();}
 catch(error) {writeFileSync(new URL('account-switch-failure.json',dir),JSON.stringify({url:switchPage.url(),body:await switchPage.locator('body').innerText(),session:await (await switchContext.request.get(app+'/api/session')).json(),datasets:await (await switchContext.request.get(app+'/api/reasoning?project_ref=project:same')).json()}),{mode:0o600});throw error;}
 check('Account switch and browser Back hide the previous graph',true);
 await switchContext.request.post(app+'/auth/signout',{headers:{origin:app}});
 await switchPage.reload();
 await expect(switchPage.getByText('PRIVATE-B-SENTINEL',{exact:true}).first()).not.toBeVisible();
 check('Logout cannot redisplay a cached private graph',true);
 await switchContext.close();
 }
 async function refresh(agent) {
  return json(metadata.token_endpoint,{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'refresh_token',client_id:agent.registration.client_id,refresh_token:agent.tokens.refresh_token,resource})});
 }
 const refreshed=await refresh(a.agent);assert.equal(refreshed.response.status,200,JSON.stringify(refreshed.body));
 check('Native refresh preserves connection generation',JSON.parse(Buffer.from(refreshed.body.access_token.split('.')[1],'base64url')).nf_connection_id===a.agent.claims.nf_connection_id);
 a.agent.tokens=refreshed.body;
 const old=a.agent;
 checked(await a.db.rpc('disconnect_agent',{connection_id:old.claims.nf_connection_id}));
 const humanAuth=createClient(base,key,{auth:{persistSession:false,autoRefreshToken:false}});
 checked(await humanAuth.auth.setSession(a.session));
 checked(await humanAuth.auth.oauth.revokeGrant({clientId:old.registration.client_id}));
 check('Disconnect blocks existing native access token',checked(await client(old.tokens.access_token).from('nodes').select('*')).length===0);
 check('Disconnect blocks native refresh token',(await refresh(old)).response.status!==200);
 const reconnect=await authorize(a,false,old.registration);
 check('Reconnect uses a new connection generation',reconnect.claims.nf_connection_id!==old.claims.nf_connection_id);
 check('Reconnect cannot restore old access token',checked(await client(old.tokens.access_token).from('nodes').select('*')).length===0);
 check('Read-only reconnect cannot write',Boolean((await client(reconnect.tokens.access_token).rpc('capture_oauth_node',{target_node_id:'D-denied',target_type:'Decision',target_status:'candidate',target_title:'Denied',target_summary:'',target_body:'',target_scope_type:null,target_scope_refs:[],target_confidence:null,target_reversibility:null,target_consequence:null,target_reconsider_conditions:[],target_metadata:{},target_project_id:a.project.id})).error));
 // A third identity joins A's team through a targeted invitation. It is not an owner/admin.
 const member={id:randomUUID(),email:`native-member-${randomUUID()}@example.test`,password:randomBytes(32).toString('base64url'),label:'Team member',workspace:a.workspace};
 sql(`insert into auth.users(id,instance_id,aud,role,email,encrypted_password,email_confirmed_at,created_at,updated_at,confirmation_token,recovery_token,email_change,email_change_token_new,raw_app_meta_data,raw_user_meta_data)
 values(${literal(member.id)},'00000000-0000-0000-0000-000000000000','authenticated','authenticated',${literal(member.email)},extensions.crypt(${literal(member.password)},extensions.gen_salt('bf')),now(),now(),now(),'','','','','{"provider":"google","providers":["google"]}','{}');
 insert into auth.identities(id,user_id,provider_id,provider,identity_data,created_at,updated_at)
 values(gen_random_uuid(),${literal(member.id)},${literal(member.id)},'google',jsonb_build_object('sub',${literal(member.id)},'email',${literal(member.email)},'email_verified',true),now(),now());`);
 const memberAuth=createClient(base,key,{auth:{persistSession:false,autoRefreshToken:false}});
 member.session=checked(await memberAuth.auth.signInWithPassword({email:member.email,password:member.password})).session;
 member.db=client(member.session.access_token);
 const memberInvite=checked(await a.db.rpc('create_workspace_invitation',{invited_email:member.email}));
 const memberRedemptions=await Promise.all([member.db.rpc('redeem_beta_invitation',{invitation_id:memberInvite.id,invitation_secret:memberInvite.secret}),member.db.rpc('redeem_beta_invitation',{invitation_id:memberInvite.id,invitation_secret:memberInvite.secret})]);
 check('Concurrent team join resolves same workspace',memberRedemptions.every(r=>checked(r)[0].workspace_id===a.workspace));
 check('Joined identity has reviewer context',checked(await member.db.rpc('get_workspace_context'))[0].membership_role==='reviewer');
 check('Reviewer reads shared graph with unfiltered REST',checked(await member.db.from('nodes').select('*')).every(n=>n.workspace_id===a.workspace));
 check('Reviewer cannot invite teammates',Boolean((await member.db.rpc('create_workspace_invitation',{invited_email:`denied-${randomUUID()}@example.test`})).error));
 check('Reviewer cannot become platform administrator',checked(await member.db.rpc('is_beta_admin'))===false);
 check('Reviewer cannot create second workspace',checked(await member.db.rpc('bootstrap_personal_workspace'))[0].workspace_id===a.workspace);
 const memberCookies=[];const memberSsr=createServerClient(base,key,{cookies:{getAll:()=>[],setAll:values=>memberCookies.push(...values)}});
 checked(await memberSsr.auth.setSession(member.session));
 member.context=await browser.newContext();await member.context.addCookies(memberCookies.map(c=>({name:c.name,value:c.value,url:app,httpOnly:false,sameSite:'Lax'})));
 member.page=await member.context.newPage();member.agent=await authorize(member);
 const memberAgent=client(member.agent.tokens.access_token);
 check('Reviewer native OAuth reads team workspace',checked(await memberAgent.from('projects').select('*')).every(p=>p.workspace_id===a.workspace));
 const memberNode=checked(await memberAgent.rpc('capture_oauth_node',{target_node_id:'D-team-member',target_type:'Decision',target_status:'candidate',target_title:'Shared team proposal',target_summary:'',target_body:'',target_scope_type:null,target_scope_refs:[],target_confidence:null,target_reversibility:null,target_consequence:null,target_reconsider_conditions:['Changed'],target_metadata:{},target_project_id:a.project.id})).node;
 check('Reviewer owns an independent agent attribution',memberNode.created_by_actor_id!==checked(await a.db.from('nodes').select('created_by_actor_id').eq('id','D-same').single()).created_by_actor_id);
 const memberProject=checked(await member.db.rpc('save_project',{target_workspace_id:a.workspace,target_project_id:randomUUID(),project_ref:'project:member-created',project_name:'Member created',project_description:'',parent_project_id:a.project.id}));
 checked(await member.db.rpc('organize_node',{target_workspace_id:a.workspace,target_node_id:memberNode.id,owning_project_id:memberProject.id,inheritance:'local',expected_updated_at:memberNode.updated_at}));
 const stale=await member.db.rpc('ratify_node',{target_workspace_id:a.workspace,target_node_id:memberNode.id,supplied_reconsider_conditions:['Changed'],expected_updated_at:memberNode.updated_at});
 check('Stale collaborative review is rejected',stale.error?.message==='STALE_REVIEW');
 const freshNode=checked(await member.db.from('nodes').select('*').eq('id',memberNode.id).single());
 const memberReview=checked(await member.db.rpc('ratify_node',{target_workspace_id:a.workspace,target_node_id:memberNode.id,supplied_reconsider_conditions:['Changed'],expected_updated_at:freshNode.updated_at}));
 check('Reviewer human review records exact identity',memberReview.ratified_by_user_id===member.id&&Boolean(memberReview.ratified_by_actor_id));
 // All preceding node writes are deliberate pre-cutover fixtures. Exercise the
 // production migration boundary before relying on the native application.
 importLocalLegacy(fixtures);
 a.agent=await authorize(a,true);
 for(const u of fixtures) {
  await nativeDiscovery(u);
  const search=await mcpCall(u.agent,'search_reasoning',{project_ref:'project:same',query:'PRIVATE',limit:1});
  check(`User ${u.label}: signed native search is bounded and workspace-private`,search.response.status===200&&!search.call.isError&&search.value.matches.length===1&&JSON.stringify(search.value).includes(`PRIVATE-${u.label}-SENTINEL`)&&!JSON.stringify(search.value).includes(`PRIVATE-${u.label==='A'?'B':'A'}-SENTINEL`));
  const brief=await mcpCall(u.agent,'get_reasoning_brief',{project_ref:'project:child',task:'Inherited private guidance'});
  check(`User ${u.label}: native brief retains approved ancestor guidance`,brief.response.status===200&&!brief.call.isError&&brief.value.complete&&brief.value.coverage_complete&&JSON.stringify(brief.value).includes(`PRIVATE-${u.label}-SENTINEL`)&&!JSON.stringify(brief.value).includes(`PRIVATE-${u.label==='A'?'B':'A'}-SENTINEL`));
  const refreshed=await mcpCall(u.agent,'get_reasoning_brief',{project_ref:'project:child',task:'Inherited private guidance',previous_fingerprint:brief.value.fingerprint});
  check(`User ${u.label}: unchanged native brief fingerprint is stable`,refreshed.value.changed===false);
  const api=await u.context.request.get(app+'/api/reasoning?'+new URLSearchParams({project_ref:'project:same',dataset_id:datasetArgs(u).target_dataset_id}));
  const apiText=await api.text();
  check(`User ${u.label}: authenticated reasoning bundle remains private and uncached`,api.status()===200&&apiText.includes(`PRIVATE-${u.label}-SENTINEL`)&&!apiText.includes(`PRIVATE-${u.label==='A'?'B':'A'}-SENTINEL`)&&api.headers()['cache-control'].includes('private, no-store'));
  const stored=await readNative(u),context=checked(await client(u.agent.tokens.access_token).rpc('agent_request_context'));
  const actor={id:'agent:'+context.actor.id,name:context.actor.display_name,kind:'agent'};
  const text=`PRIVATE-${u.label}-NATIVE qualified observation: this synthetic result does not establish product reliability.`;
  const delta=nativeDelta(stored,actor,text),args={project_ref:'project:same',dataset_id:stored.bundle.id,delta,expected_head:stored.head,operation_id:randomUUID()};
  const captured=await mcpCall(u.agent,'capture_reasoning',args),repeated=await mcpCall(u.agent,'capture_reasoning',args);
  check(`User ${u.label}: native incremental capture succeeds without authority`,!captured.call.isError&&captured.value.authorityChanged===false);
  assert.deepEqual(repeated.value,captured.value,'Idempotent native capture must preserve exact result');
  const expanded=await mcpCall(u.agent,'expand_reasoning',{project_ref:'project:same',dataset_id:stored.bundle.id,record_id:delta.entries[0].id});
  check(`User ${u.label}: native readback preserves exact qualified text and authenticated recorder`,expanded.value.entry.content===text&&expanded.value.sources[0].content===text&&expanded.value.serverCapture.actorId===actor.id&&expanded.value.commitments.length===0);
  const updated=await readNative(u);
  check(`User ${u.label}: native retry adds exactly one contribution`,updated.bundle.entries.length===stored.bundle.entries.length+1&&updated.commitments.length===stored.commitments.length);
  const historical=await mcpCall(u.agent,'get_reconstruction_packet',{project_ref:'project:same',dataset_id:stored.bundle.id,run_id:stored.bundle.runs.at(-1).id,entry_ids:[stored.bundle.entries.find(entry=>entry.logicalId==='legacy:nodes:D-same').id]});
  check(`User ${u.label}: historical native packet excludes later capture`,!historical.call.isError&&!JSON.stringify(historical.value).includes(text)&&JSON.stringify(historical.value).includes(`PRIVATE-${u.label}-SENTINEL`));
  const denied=await mcpCall(u.agent,'change_commitment',{project_ref:'project:same',dataset_id:stored.bundle.id,action:'adopt',entry_id:delta.entries[0].id,prior_receipt_id:null,rationale:'Unassigned agent must not gain authority from capture.',expected_head:updated.head,operation_id:randomUUID()});
  check(`User ${u.label}: ungranted native agent commitment denied`,denied.call.isError===true&&JSON.stringify(denied.call).includes('FORBIDDEN'));
  const foreign=fixtures.find(other=>other!==u);
  for(const db of [u.db,client(u.agent.tokens.access_token)]) {
   for(const table of ['reasoning_datasets','reasoning_records','reasoning_revisions','reasoning_commitments','reasoning_authority_events']) check(`User ${u.label}: native ${table} RLS isolation (${report.checks.length})`,checked(await db.from(table).select('workspace_id')).every(row=>row.workspace_id===u.workspace));
   check(`User ${u.label}: foreign native dataset denied (${report.checks.length})`,Boolean((await db.rpc('reasoning_read_bundle',datasetArgs(foreign))).error));
  }
  check(`User ${u.label}: retired capture RPC blocked after cutover`,Boolean((await client(u.agent.tokens.access_token).rpc('capture_oauth_node',{target_node_id:'D-post-cutover-denied',target_type:'Decision',target_status:'candidate',target_title:'Denied',target_summary:'',target_body:'',target_scope_type:null,target_scope_refs:[],target_confidence:null,target_reversibility:null,target_consequence:null,target_reconsider_conditions:[],target_metadata:{},target_project_id:u.project.id})).error));
 }
 const readonlyStored=await readNative(a),readonlyContext=checked(await client(reconnect.tokens.access_token).rpc('agent_request_context'));
 const readonlyDelta=nativeDelta(readonlyStored,{id:'agent:'+readonlyContext.actor.id,name:'Read-only fixture',kind:'agent'},'Read-only denial fixture');
 const readonlyCapture=await mcpCall(reconnect,'capture_reasoning',{project_ref:'project:same',dataset_id:readonlyStored.bundle.id,delta:readonlyDelta,expected_head:readonlyStored.head,operation_id:randomUUID()});
 check('Read-only OAuth connection cannot capture native reasoning',readonlyCapture.call.isError===true&&JSON.stringify(readonlyCapture.call).includes('FORBIDDEN'));
 const nativeRefreshed=await refresh(a.agent);assert.equal(nativeRefreshed.response.status,200);
 const nativeClaims=JSON.parse(Buffer.from(nativeRefreshed.body.access_token.split('.')[1],'base64url'));
 check('Post-cutover native refresh preserves exact audience and connection generation',nativeClaims.aud===resource&&nativeClaims.nf_connection_id===a.agent.claims.nf_connection_id);
 a.agent.tokens=nativeRefreshed.body;
 check('Refreshed OAuth access token reads the native reasoning service',!(await mcpCall(a.agent,'list_reasoning_datasets',{project_ref:'project:same'})).call.isError);
 const peerBefore=await readNative(member,memberProject),peerContext=checked(await memberAgent.rpc('agent_request_context'));
 const peerActor={id:'agent:'+peerContext.actor.id,name:peerContext.actor.display_name,kind:'agent'},peerDelta=nativeDelta(peerBefore,peerActor,'Shared native team proposal; original human decision remains inspectable.');
 const peerCaptured=await mcpCall(member.agent,'capture_reasoning',{project_ref:memberProject.ref,dataset_id:peerBefore.bundle.id,delta:peerDelta,expected_head:peerBefore.head,operation_id:randomUUID()});
 check('Reviewer-owned native agent records independent attribution',!peerCaptured.call.isError&&(await readNative(member,memberProject)).captureMetadata[peerDelta.entries[0].id].actorId===peerActor.id);
 const commitmentArgs={...datasetArgs(member,memberProject),action:'adopt',contribution_id:peerDelta.entries[0].id,prior_receipt_id:null,rationale:'Synthetic teammate approval test',operation_id:randomUUID()};
 const nativeStale=await member.db.rpc('reasoning_commit',{...commitmentArgs,expected_head:peerBefore.head});
 check('Stale native teammate commitment is rejected',Boolean(nativeStale.error)&&nativeStale.error.message.includes('STALE'));
 const peerCurrent=await readNative(member,memberProject);
 checked(await member.db.rpc('reasoning_commit',{...commitmentArgs,expected_head:peerCurrent.head}));
 const peerApproved=await readNative(member,memberProject),peerReceipt=peerApproved.commitments.find(receipt=>receipt.entryId===peerDelta.entries[0].id);
 check('Reviewer native commitment retains exact human identity',peerReceipt.actorId==='human:'+member.id);
 check('Reviewer cannot grant native agent authority',Boolean((await member.db.rpc('reasoning_grant_authority',{target_workspace_id:a.workspace,target_project_id:memberProject.id,target_connection_id:member.agent.claims.nf_connection_id})).error));
 check('Reviewer standalone withdrawal remains owner-only',Boolean((await member.db.rpc('reasoning_commit',{...commitmentArgs,action:'withdraw',prior_receipt_id:peerReceipt.id,expected_head:peerApproved.head,operation_id:randomUUID()})).error));
 const grant=checked(await a.db.rpc('reasoning_grant_authority',{target_workspace_id:a.workspace,target_project_id:memberProject.id,target_connection_id:member.agent.claims.nf_connection_id}));
 const revisionDelta=nativeDelta(peerApproved,peerActor,'Granted agent revision of the synthetic human commitment; originals retained.');
 const revisionCapture=await mcpCall(member.agent,'capture_reasoning',{project_ref:memberProject.ref,dataset_id:peerApproved.bundle.id,delta:revisionDelta,expected_head:peerApproved.head,operation_id:randomUUID()});
 const revision=await mcpCall(member.agent,'change_commitment',{project_ref:memberProject.ref,dataset_id:peerApproved.bundle.id,action:'revise',entry_id:revisionDelta.entries[0].id,prior_receipt_id:peerReceipt.id,rationale:'Explicit disposable project grant covers this revision.',expected_head:revisionCapture.value.head,operation_id:randomUUID()});
 check('Named agent may revise a human commitment with a current project grant',!revision.call.isError);
 const peerRevised=await readNative(member,memberProject),agentReceipt=peerRevised.commitments.find(receipt=>receipt.entryId===revisionDelta.entries[0].id);
 assert.deepEqual(peerRevised.commitments.find(receipt=>receipt.id===peerReceipt.id),peerReceipt,'Human approval receipt must remain unchanged');
 assert.deepEqual(peerRevised.bundle.entries.find(entry=>entry.id===peerDelta.entries[0].id),peerApproved.bundle.entries.find(entry=>entry.id===peerDelta.entries[0].id),'Original human-approved contribution must remain unchanged');
 check('Granted revision records the exact prior human receipt and grant',agentReceipt.priorReceiptId===peerReceipt.id&&agentReceipt.grantId===grant.id);
 checked(await a.db.rpc('reasoning_revoke_authority',{target_workspace_id:a.workspace,target_project_id:memberProject.id,target_grant_id:grant.id}));
 const revoked=await mcpCall(member.agent,'change_commitment',{project_ref:memberProject.ref,dataset_id:peerRevised.bundle.id,action:'withdraw',entry_id:agentReceipt.entryId,prior_receipt_id:agentReceipt.id,rationale:'Revoked grant must deny future action.',expected_head:peerRevised.head,operation_id:randomUUID()});
 check('Revoked native grant blocks future actions without changing past receipts',revoked.call.isError===true&&(await readNative(member,memberProject)).commitments.some(receipt=>receipt.id===agentReceipt.id));
 await verifyAccountSwitch();
 check('Reviewer cannot remove owner',Boolean((await member.db.rpc('remove_workspace_member',{target_user_id:a.id})).error));
 check('Owner cannot remove sole owner',Boolean((await a.db.rpc('remove_workspace_member',{target_user_id:a.id})).error));
 checked(await a.db.rpc('remove_workspace_member',{target_user_id:member.id}));
 check('Removed member human JWT cannot expand hosted reasoning',Boolean((await member.db.rpc('reasoning_read_bundle',datasetArgs(member,memberProject))).error));
 check('Removed member OAuth cannot discover native tools',(await mcpCall(member.agent,'list_projects',{})).response.status===401);
 check('Removed member existing human JWT cannot read',checked(await member.db.from('nodes').select('*')).length===0);
 check('Removed member cannot bootstrap fresh workspace',Boolean((await member.db.rpc('bootstrap_personal_workspace')).error));
 check('Removed member agent access fails',checked(await memberAgent.from('nodes').select('*')).length===0);
 check('Removed member native refresh fails',(await refresh(member.agent)).response.status!==200);
 check('Removed member audit attribution is retained',checked(await a.db.from('nodes').select('ratified_by_user_id').eq('id',memberNode.id).single()).ratified_by_user_id===member.id);
 await member.context.close();
 checked(await a.db.rpc('set_account_status',{target_user_id:b.id,new_status:'suspended'}));
 check('Suspension blocks native reasoning with existing human JWT',Boolean((await b.db.rpc('reasoning_read_bundle',datasetArgs(b))).error));
 check('Suspension blocks existing native agent access',(await mcpCall(b.agent,'list_projects',{})).response.status===401);
 check('Suspension blocks existing human JWT',checked(await b.db.from('nodes').select('*')).length===0);
 check('Suspension blocks agent refresh',(await refresh(b.agent)).response.status!==200);
 report.result='passed';
 // Local sessions support a separate, explicit Codex CLI/browser test. Never printed.
 writeFileSync(new URL('fixtures.json',dir),JSON.stringify(fixtures.map(({id,email,password,session,workspace,project,agent})=>({id,email,password,session,workspace,project,agent}))),{mode:0o600});
} catch(error) {report.result='failed';report.error=error.message;console.error(error.message);process.exitCode=1;}
finally {await browser?.close();report.finishedAt=new Date().toISOString();writeFileSync(new URL('native-report.json',dir),JSON.stringify(report,null,2),{mode:0o600});console.log(JSON.stringify({result:report.result,checks:report.checks.length,report:fileURLToPath(new URL('native-report.json',dir))}));}
