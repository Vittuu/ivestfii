import type { InvestmentProfile, InvestmentProfileInput } from './bridge';

export interface OnboardingStatus {
  autoShow: boolean;
  walletId: string;
  step: number;
  status: 'not_started' | 'started' | 'skipped' | 'completed' | 'existing';
}

/** Preserve all declared goals, dates and preferences outside these two answers. */
export function initialProfile(profile: InvestmentProfile | null, riskTolerance: InvestmentProfileInput['riskTolerance'], emergencyReserveStatus: InvestmentProfileInput['emergencyReserveStatus']): InvestmentProfileInput {
  return {
    riskTolerance, emergencyReserveStatus,
    shortTermObjective: profile?.shortTermObjective ?? '', shortTermMonths: profile?.shortTermMonths ?? null,
    longTermObjective: profile?.longTermObjective ?? '', longTermMonths: profile?.longTermMonths ?? null,
    liquidityNeeds: profile?.liquidityNeeds ?? '', monthlyContribution: profile?.monthlyContribution ?? null,
    knowledgeLevel: profile?.knowledgeLevel ?? null, constraints: profile?.constraints ?? '', notes: profile?.notes ?? '',
  };
}

export const onboardingHelp = 'Estou começando a usar o IvestFII. Explique de forma simples como organizar minha carteira e registrar meus investimentos. Tire minhas dúvidas com poucas perguntas por vez. Antes de sugerir investimentos, pergunte o que faltar sobre meus objetivos, prazos e perfil. Não presuma que meu perfil está completo nem que minhas respostas foram salvas; indique onde posso cadastrá-las no aplicativo.';
