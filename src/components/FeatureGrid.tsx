interface FeatureItem {
  title: string;
  description: string;
}

interface FeatureGridProps {
  features: FeatureItem[];
}

export const FeatureGrid = ({ features }: FeatureGridProps) => (
  <section className="feature-grid" aria-label="Planned scaffold capabilities">
    {features.map((feature) => (
      <article className="feature-card" key={feature.title}>
        <h2>{feature.title}</h2>
        <p>{feature.description}</p>
      </article>
    ))}
  </section>
);
