import { useEffect, useRef, useState } from "react";
import { Textarea } from "@/components/ui/textarea";
export function SyncedTextarea({ value, onValueChange, className, }: {
    value: string;
    onValueChange: (value: string) => void;
    className?: string;
}) {
    const [localValue, setLocalValue] = useState(value);
    const isFocusedRef = useRef(false);
    const latestExternalValueRef = useRef(value);
    useEffect(() => {
        latestExternalValueRef.current = value;
        if (!isFocusedRef.current) {
            setLocalValue(value);
        }
    }, [value]);
    return (<Textarea className={className} value={localValue} onBlur={() => {
            isFocusedRef.current = false;
            setLocalValue(latestExternalValueRef.current);
        }} onChange={(event) => {
            const next = event.currentTarget.value;
            setLocalValue(next);
            onValueChange(next);
        }} onFocus={() => {
            isFocusedRef.current = true;
        }} spellCheck={false}/>);
}
