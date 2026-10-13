import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {spawn} from 'node:child_process';
const credentials=JSON.parse(readFileSync(new URL('../../.neurofilament/local-beta/stack.json',import.meta.url),'utf8'));
if(credentials.API_URL!=='http://127.0.0.1:55421')throw new Error('Refusing a non-local Supabase destination.');
const key=credentials.PUBLISHABLE_KEY??credentials.ANON_KEY;
if(!key)throw new Error('Local API key unavailable.');
const env={...process.env,NEXT_PUBLIC_SUPABASE_URL:credentials.API_URL,NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY:key,
  APP_URL:'http://127.0.0.1:3108',NEUROFILAMENT_DEMO_MODE:'false',SUPABASE_SECRET_KEY:'',OWNER_EMAIL:'',VERCEL_OIDC_TOKEN:''};
const child=spawn(process.execPath,['node_modules/next/dist/bin/next','dev','--hostname','127.0.0.1','--port','3108'],{
  cwd:fileURLToPath(new URL('../',import.meta.url)),env,stdio:'inherit'});
for(const signal of ['SIGTERM','SIGINT'])process.on(signal,()=>child.kill(signal));
child.on('exit',code=>process.exit(code??1));
