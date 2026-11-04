import {
    isFamilyAdmin,
    isPartOfFamily,
    type UserDetails,
} from "ente-new/photos/services/user-details";

export const openManageSubscription = ({
    userDetails,
    showManageMemberSubscription,
    onShowPlanSelector,
}: {
    userDetails: UserDetails | undefined;
    showManageMemberSubscription: () => void;
    onShowPlanSelector: () => void;
}) => {
    if (
        userDetails &&
        isPartOfFamily(userDetails) &&
        !isFamilyAdmin(userDetails)
    ) {
        showManageMemberSubscription();
    } else {
        // PHOTOVAULT: strip - no Stripe customer portal. The caller passes
        // family management as `onShowPlanSelector`.
        onShowPlanSelector();
    }
};
