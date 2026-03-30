import { SpeedwagonDetail } from "@/components/speedwagons/speedwagon-detail";

interface Props {
  params: Promise<{ id: string }>;
}

export default async function SpeedwagonPage({ params }: Props) {
  const { id } = await params;
  return <SpeedwagonDetail id={id} />;
}
