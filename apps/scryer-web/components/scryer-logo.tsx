type ScryerLogoProps = {
  className?: string;
};

const logoUrl = `${import.meta.env.BASE_URL}logo.svg`;

export default function ScryerLogo({ className = "" }: ScryerLogoProps) {
  return (
    <div data-slot="scryer-logo" className={`flex h-20 w-20 items-center ${className}`}>
      <img
        src={logoUrl}
        alt="Scryer Logo"
        data-slot="scryer-logo-image"
        className="h-full w-full object-contain"
      />
    </div>
  );
}
