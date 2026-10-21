import { AppShell } from '@/components/app-shell';
import ReasoningWorkspace from '@/components/reasoning/ReasoningWorkspace';

export const dynamic = 'force-dynamic';
export default async function GraphPage({ searchParams }: { searchParams: Promise<{ project?: string }> }) {
  const { project } = await searchParams;
  return <AppShell active="Reasoning"><ReasoningWorkspace mode="hosted" initialProject={project ?? ''} /></AppShell>;
}
