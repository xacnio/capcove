import legal from "../data/legal.json";
import LegalPage from "../components/LegalPage.jsx";

export default function TermsPage() {
  return <LegalPage title="Terms of Service" doc={legal.terms} />;
}
