import { cookies } from 'next/headers';
import { PROJECT_COOKIE, readProjectCookie } from '@/lib/projects';
import { AppShell } from '@/components/app-shell';
import ReasoningWorkspace from '@/components/reasoning/ReasoningWorkspace';
import { requireOwner } from '@/lib/server/auth';

export const dynamic = 'force-dynamic';
export default async function GraphPage({ searchParams }: { searchParams: Promise<{ project?: string }> }) {
  const [, params, cookieStore] = await Promise.all([requireOwner(), searchParams, cookies()]);
  const project = params.project ?? readProjectCookie(cookieStore.get(PROJECT_COOKIE)?.value);
  return <AppShell active="Reasoning"><ReasoningWorkspace mode="hosted" initialProject={project === 'all' ? '' : project} /></AppShell>;
}
