import type { Metadata } from 'next';
import ReasoningWorkspace from '@/components/reasoning/ReasoningWorkspace';

export const metadata: Metadata = { title: 'Inspect a bundle · Loomtracer', description: 'Inspect a reasoning bundle privately in your browser. Files are uploaded only when you explicitly save them to a project.' };
export default function InspectPage() { return <ReasoningWorkspace mode="local" />; }
