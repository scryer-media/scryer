type ScryerLogoProps = {
  className?: string;
};

const logoUrl = `${import.meta.env.BASE_URL}logo.svg`;

export default function ScryerLogo({ className = "" }: ScryerLogoProps) {
  return (
    <div className={`flex h-20 w-20 items-center ${className}`}>
      <img
        src={logoUrl}
        alt="Scryer Logo"
        className="h-full w-full object-contain"
      />
    </div>
  );
}
