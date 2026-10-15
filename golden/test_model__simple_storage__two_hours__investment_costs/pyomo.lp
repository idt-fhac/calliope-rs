\* Source Pyomo model name=None *\

min 
objectives(min_cost_optimisation)(0):
+0.00012576263948632513 variables(flow_cap)(a__test_link_a_b_elec__electricity)
+0.00012576263948632513 variables(flow_cap)(a__test_link_a_b_heat__heat)
+0.00025152527897265027 variables(flow_cap)(a__test_storage__electricity)
+0.00025152527897265027 variables(flow_cap)(a__test_supply_elec__electricity)
+0.00012576263948632513 variables(flow_cap)(b__test_link_a_b_elec__electricity)
+0.00012576263948632513 variables(flow_cap)(b__test_link_a_b_heat__heat)
+0.00025152527897265027 variables(flow_cap)(b__test_storage__electricity)
+0.00025152527897265027 variables(flow_cap)(b__test_supply_elec__electricity)

s.t.

c_u_constraints(flow_capacity_per_storage_capacity_max)(a__test_storage__electricity)_:
+1 variables(flow_cap)(a__test_storage__electricity)
-0.5 variables(storage_cap)(a__test_storage)
<= 0.0

c_u_constraints(flow_capacity_per_storage_capacity_max)(b__test_storage__electricity)_:
+1 variables(flow_cap)(b__test_storage__electricity)
-0.5 variables(storage_cap)(b__test_storage)
<= 0.0

c_u_constraints(flow_out_max)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_elec__electricity)
+1 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_elec__electricity)
+1 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_link_a_b_heat__heat__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_heat__heat)
+1 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_link_a_b_heat__heat__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_heat__heat)
+1 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_storage__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_storage__electricity)
+1 variables(flow_out)(a__test_storage__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_storage__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_storage__electricity)
+1 variables(flow_out)(a__test_storage__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_supply_elec__electricity)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(a__test_supply_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_supply_elec__electricity)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_elec__electricity)
+1 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_elec__electricity)
+1 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_link_a_b_heat__heat__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_heat__heat)
+1 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_link_a_b_heat__heat__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_heat__heat)
+1 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_storage__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_storage__electricity)
+1 variables(flow_out)(b__test_storage__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_storage__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_storage__electricity)
+1 variables(flow_out)(b__test_storage__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_supply_elec__electricity)
+1 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max)(b__test_supply_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_supply_elec__electricity)
+1 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_elec__electricity)
+1 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_elec__electricity)
+1 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_link_a_b_heat__heat__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_heat__heat)
+1 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_link_a_b_heat__heat__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_link_a_b_heat__heat)
+1 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_storage__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(a__test_storage__electricity)
+1 variables(flow_in)(a__test_storage__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(a__test_storage__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_storage__electricity)
+1 variables(flow_in)(a__test_storage__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(b__test_demand_elec__electricity)
<= 0.0

c_u_constraints(flow_in_max)(b__test_demand_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_demand_elec__electricity)
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_elec__electricity)
+1 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_elec__electricity)
+1 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_link_a_b_heat__heat__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_heat__heat)
+1 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_link_a_b_heat__heat__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_link_a_b_heat__heat)
+1 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_storage__electricity__2005_01_01_00_00)_:
-1.0 variables(flow_cap)(b__test_storage__electricity)
+1 variables(flow_in)(b__test_storage__electricity__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_in_max)(b__test_storage__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(b__test_storage__electricity)
+1 variables(flow_in)(b__test_storage__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(a__test_supply_elec__2005_01_01_00_00)_:
+1 variables(source_use)(a__test_supply_elec__2005_01_01_00_00)
-1.0 variables(source_cap)(a__test_supply_elec)
<= 0.0

c_u_constraints(source_max)(a__test_supply_elec__2005_01_01_01_00)_:
-1.0 variables(source_cap)(a__test_supply_elec)
+1 variables(source_use)(a__test_supply_elec__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(b__test_supply_elec__2005_01_01_00_00)_:
+1 variables(source_use)(b__test_supply_elec__2005_01_01_00_00)
-1.0 variables(source_cap)(b__test_supply_elec)
<= 0.0

c_u_constraints(source_max)(b__test_supply_elec__2005_01_01_01_00)_:
-1.0 variables(source_cap)(b__test_supply_elec)
+1 variables(source_use)(b__test_supply_elec__2005_01_01_01_00)
<= 0.0

c_u_constraints(storage_max)(a__test_storage__2005_01_01_00_00)_:
-1 variables(storage_cap)(a__test_storage)
+1 variables(storage)(a__test_storage__2005_01_01_00_00)
<= 0.0

c_u_constraints(storage_max)(a__test_storage__2005_01_01_01_00)_:
-1 variables(storage_cap)(a__test_storage)
+1 variables(storage)(a__test_storage__2005_01_01_01_00)
<= 0.0

c_u_constraints(storage_max)(b__test_storage__2005_01_01_00_00)_:
-1 variables(storage_cap)(b__test_storage)
+1 variables(storage)(b__test_storage__2005_01_01_00_00)
<= 0.0

c_u_constraints(storage_max)(b__test_storage__2005_01_01_01_00)_:
-1 variables(storage_cap)(b__test_storage)
+1 variables(storage)(b__test_storage__2005_01_01_01_00)
<= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
+1 variables(flow_out)(a__test_storage__electricity__2005_01_01_00_00)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(a__test_storage__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
+1 variables(flow_out)(a__test_storage__electricity__2005_01_01_01_00)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(a__test_storage__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(a__heat__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
-1 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__heat__2005_01_01_01_00)_:
+1 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
-1 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(b__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
+1 variables(flow_out)(b__test_storage__electricity__2005_01_01_00_00)
+1 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_storage__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(b__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
+1 variables(flow_out)(b__test_storage__electricity__2005_01_01_01_00)
+1 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_storage__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(system_balance)(b__heat__2005_01_01_00_00)_:
+1 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
-1 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(b__heat__2005_01_01_01_00)_:
+1 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
-1 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
= 5.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
= 5.0

c_e_constraints(balance_demand)(b__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00)
= 5.0

c_e_constraints(balance_demand)(b__test_demand_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00)
= 5.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_elec__electricity__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
-1 variables(source_use)(a__test_supply_elec__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_elec__electricity__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
-1 variables(source_use)(a__test_supply_elec__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_elec__electricity__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_00_00)
-1 variables(source_use)(b__test_supply_elec__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(b__test_supply_elec__electricity__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_01_00)
-1 variables(source_use)(b__test_supply_elec__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_storage)(a__test_storage__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(a__test_storage__electricity__2005_01_01_00_00)
-1 variables(flow_in)(a__test_storage__electricity__2005_01_01_00_00)
+1 variables(storage)(a__test_storage__2005_01_01_00_00)
-0.99 variables(storage)(a__test_storage__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_storage)(a__test_storage__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(a__test_storage__electricity__2005_01_01_01_00)
-1 variables(flow_in)(a__test_storage__electricity__2005_01_01_01_00)
-0.99 variables(storage)(a__test_storage__2005_01_01_00_00)
+1 variables(storage)(a__test_storage__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_storage)(b__test_storage__2005_01_01_00_00)_:
+1.1111111111111112 variables(flow_out)(b__test_storage__electricity__2005_01_01_00_00)
-1 variables(flow_in)(b__test_storage__electricity__2005_01_01_00_00)
+1 variables(storage)(b__test_storage__2005_01_01_00_00)
-0.99 variables(storage)(b__test_storage__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_storage)(b__test_storage__2005_01_01_01_00)_:
+1.1111111111111112 variables(flow_out)(b__test_storage__electricity__2005_01_01_01_00)
-1 variables(flow_in)(b__test_storage__electricity__2005_01_01_01_00)
-0.99 variables(storage)(b__test_storage__2005_01_01_00_00)
+1 variables(storage)(b__test_storage__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_transmission)(test_link_a_b_elec__2005_01_01_00_00)_:
+1.0 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
+1.0 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_transmission)(test_link_a_b_elec__2005_01_01_01_00)_:
+1.0 variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
+1.0 variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
-1.0 variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_01_00)
-1.0 variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_transmission)(test_link_a_b_heat__2005_01_01_00_00)_:
+1.0 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
+1.0 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
-1.0 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_00_00)
-1.0 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_transmission)(test_link_a_b_heat__2005_01_01_01_00)_:
+1.0 variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
+1.0 variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
-1.0 variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_01_00)
-1.0 variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_01_00)
= 0.0

c_e_constraints(symmetric_transmission)(test_link_a_b_elec__electricity)_:
+1 variables(flow_cap)(a__test_link_a_b_elec__electricity)
-1 variables(flow_cap)(b__test_link_a_b_elec__electricity)
= 0.0

c_e_constraints(symmetric_transmission)(test_link_a_b_heat__heat)_:
+1 variables(flow_cap)(a__test_link_a_b_heat__heat)
-1 variables(flow_cap)(b__test_link_a_b_heat__heat)
= 0.0

bounds
   0.0 <= variables(flow_cap)(a__test_link_a_b_elec__electricity) <= 10.0
   0.0 <= variables(flow_cap)(a__test_link_a_b_heat__heat) <= 5.0
   0.0 <= variables(flow_cap)(a__test_storage__electricity) <= 10.0
   0.0 <= variables(flow_cap)(a__test_supply_elec__electricity) <= 10.0
   0.0 <= variables(flow_cap)(b__test_link_a_b_elec__electricity) <= 10.0
   0.0 <= variables(flow_cap)(b__test_link_a_b_heat__heat) <= 5.0
   0.0 <= variables(flow_cap)(b__test_storage__electricity) <= 10.0
   0.0 <= variables(flow_cap)(b__test_supply_elec__electricity) <= 10.0
   0.0 <= variables(storage_cap)(a__test_storage) <= 15.0
   0.0 <= variables(storage_cap)(b__test_storage) <= 15.0
   0.0 <= variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_link_a_b_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_link_a_b_heat__heat__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_storage__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_storage__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(b__test_link_a_b_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(b__test_link_a_b_heat__heat__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_storage__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(b__test_storage__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(b__test_supply_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_demand_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(a__test_link_a_b_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(a__test_link_a_b_heat__heat__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(a__test_storage__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(a__test_storage__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(b__test_demand_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(b__test_demand_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(b__test_link_a_b_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(b__test_link_a_b_heat__heat__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_in)(b__test_storage__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_in)(b__test_storage__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(a__test_supply_elec__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(a__test_supply_elec) <= +inf
   0.0 <= variables(source_use)(a__test_supply_elec__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(b__test_supply_elec__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(b__test_supply_elec) <= +inf
   0.0 <= variables(source_use)(b__test_supply_elec__2005_01_01_01_00) <= +inf
   0.0 <= variables(storage)(a__test_storage__2005_01_01_00_00) <= +inf
   0.0 <= variables(storage)(a__test_storage__2005_01_01_01_00) <= +inf
   0.0 <= variables(storage)(b__test_storage__2005_01_01_00_00) <= +inf
   0.0 <= variables(storage)(b__test_storage__2005_01_01_01_00) <= +inf
end
