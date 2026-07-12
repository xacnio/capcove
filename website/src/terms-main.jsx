import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import { LanguageProvider } from "./lib/LanguageContext.jsx";
import TermsPage from "./pages/TermsPage.jsx";

createRoot(document.getElementById("root")).render(
  <StrictMode>
    <LanguageProvider>
      <TermsPage />
    </LanguageProvider>
  </StrictMode>
);
