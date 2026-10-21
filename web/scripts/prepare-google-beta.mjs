/** Invite the existing owner's Google identity into the disposable local test. */
import {readFileSync,writeFileSync} from 'node:fs';
import {parseEnv} from 'node:util';
import {createClient} from '@supabase/supabase-js';
const dir=new URL('../../.neurofilament/local-beta/',import.meta.url);
const credentials=JSON.parse(readFileSync(new URL('stack.json',dir),'utf8'));
if(credentials.API_URL!=='http://127.0.0.1:55421')throw new Error('Refusing non-local target.');
// Read only the existing account address; no hosted API credentials are used.
const email=(process.argv[2]??parseEnv(readFileSync(new URL('../.env.local',import.meta.url),'utf8')).OWNER_EMAIL)?.trim();
if(!email)throw new Error('Existing Google account email is unavailable.');
const [admin]=JSON.parse(readFileSync(new URL('fixtures.json',dir),'utf8'));
const db=createClient(credentials.API_URL,credentials.PUBLISHABLE_KEY??credentials.ANON_KEY,{
 accessToken:async()=>admin.session.access_token,auth:{persistSession:false,autoRefreshToken:false}});
const {data,error}=await db.rpc('create_beta_invitation',{invited_email:email});
if(error)throw new Error(error.message);
const link=`http://127.0.0.1:3108/join#${data.id}.${data.secret}`;
writeFileSync(new URL('google-invitation.json',dir),JSON.stringify({email,id:data.id,link}),{mode:0o600});
console.log(link);
