const logos = import.meta.glob('/src/assets/provider-logos/*.svg', { eager: true, import: 'default' });

export function getProviderLogo(providerId: string): string {
  const logoPath = `/src/assets/provider-logos/${providerId}.svg`;
  return (logos[logoPath] as string) || (logos['/src/assets/provider-logos/custom.svg'] as string);
}