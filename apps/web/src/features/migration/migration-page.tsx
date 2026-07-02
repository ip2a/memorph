import { useLocation, useParams, useSearchParams } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { PageEmpty } from "@/components/shared/page-states";

type MigrationPageProps = {
  title: string;
  description: string;
  legacySource: string;
  workflows: string[];
};

export function MigrationPage({ title, description, legacySource, workflows }: MigrationPageProps) {
  const location = useLocation();
  const params = useParams();
  const [searchParams] = useSearchParams();
  const query = Array.from(searchParams.entries());

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="flex flex-col gap-1">
              <CardTitle>{title}</CardTitle>
              <CardDescription>{description}</CardDescription>
            </div>
            <Badge variant="secondary">Migration target</Badge>
          </div>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-3 md:grid-cols-3">
            <div className="flex flex-col gap-1 rounded-lg border p-3">
              <span className="text-xs text-muted-foreground">Current route</span>
              <span className="truncate text-sm font-medium">{location.pathname}</span>
            </div>
            <div className="flex flex-col gap-1 rounded-lg border p-3">
              <span className="text-xs text-muted-foreground">Legacy source</span>
              <span className="truncate text-sm font-medium">{legacySource}</span>
            </div>
            <div className="flex flex-col gap-1 rounded-lg border p-3">
              <span className="text-xs text-muted-foreground">Status</span>
              <span className="text-sm font-medium">Ready for feature implementation</span>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            {workflows.map((workflow) => (
              <Badge key={workflow} variant="outline">
                {workflow}
              </Badge>
            ))}
          </div>
        </CardContent>
      </Card>

      {(Object.keys(params).length > 0 || query.length > 0) && (
        <Card size="sm">
          <CardHeader>
            <CardTitle>Route context</CardTitle>
            <CardDescription>Values preserved from the legacy route contract.</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2 text-sm">
            {Object.entries(params).map(([key, value]) => (
              <div key={key} className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">{key}</span>
                <span className="truncate font-medium">{value}</span>
              </div>
            ))}
            {query.map(([key, value]) => (
              <div key={key} className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">{key}</span>
                <span className="truncate font-medium">{value}</span>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      <PageEmpty
        title="Feature body pending migration"
        description="This React route is in place; the next step is replacing the matching legacy DOM workflow with typed queries and shadcn components."
      />
    </div>
  );
}
